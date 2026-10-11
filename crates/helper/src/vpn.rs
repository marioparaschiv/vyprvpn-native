use std::error::Error;
use std::future::Future;
use std::mem;
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use ipc::{Credentials, Status};
use tokio::sync::{Notify, OnceCell, watch};
use tokio::task::JoinError;
use tunnel::{
    AdapterPlan, ConfigError, ConnectResponse, EndpointError, KeyError, PrivateKey, PublicKey,
    TunnelConfig,
};

use crate::session_store::{Saved, Session, SessionStore, SessionStoreError};

mod recovery;

/// The VyprVPN calls that open and close a session.
pub trait SessionApi: Send + Sync + 'static {
    fn open(
        &self,
        credentials: &Credentials,
        spoke_id: &str,
        client_key: &PublicKey,
    ) -> impl Future<Output = Result<ConnectResponse, vypr_api::Error>> + Send;

    fn close(&self, session: &Session) -> impl Future<Output = Result<(), vypr_api::Error>> + Send;
}

impl SessionApi for vypr_api::Client {
    async fn open(
        &self,
        credentials: &Credentials,
        spoke_id: &str,
        client_key: &PublicKey,
    ) -> Result<ConnectResponse, vypr_api::Error> {
        let realm = self.settings(credentials).await?.realm;

        self.connect(credentials, &realm, spoke_id, client_key)
            .await
    }

    async fn close(&self, session: &Session) -> Result<(), vypr_api::Error> {
        self.disconnect(
            &session.spoke_id,
            &session.connection_id,
            &session.preshared_key,
        )
        .await
    }
}

/// The OS tunnel backend, provided by the platform module. Every call blocks, so they run on the
/// blocking pool.
pub trait Tunnel: Send + Sync + 'static {
    type Active: Send + 'static;
    type Error: Error + Send + Sync + 'static;

    /// Undoes whatever the tunnel of a previous helper left behind. Runs once, before the first
    /// `up`.
    fn recover(&self) -> Result<(), Self::Error>;

    fn up(&self, plan: &AdapterPlan) -> Result<Self::Active, Self::Error>;

    fn down(&self, active: Self::Active) -> Result<(), Self::Error>;
}

#[derive(Debug, thiserror::Error)]
enum StepError<E: Error + 'static> {
    #[error(transparent)]
    Api(#[from] vypr_api::Error),
    #[error("generating the WireGuard key failed")]
    Key(#[from] KeyError),
    #[error("VyprVPN sent an unusable tunnel config")]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Endpoint(#[from] EndpointError),
    #[error("bringing the tunnel up failed")]
    Up(#[source] E),
    #[error("taking the tunnel down failed")]
    Down(#[source] E),
    #[error("saving the connection failed")]
    Store(#[from] SessionStoreError),
    #[error("the tunnel task failed")]
    Blocking(#[from] JoinError),
}

/// The error and its causes on one line, for the tray app's notification.
fn describe(error: &dyn Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();

    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.contains(&text) {
            message = format!("{message}: {text}");
        }
        source = cause.source();
    }

    message
}

/// The location the user asked for.
#[derive(Clone)]
struct Target {
    spoke_id: String,
    credentials: Credentials,
}

struct Up<A> {
    active: A,
    spoke_id: String,
    tunnel_ip: IpAddr,
    connected_at: u64,
}

impl<A> Up<A> {
    fn status(&self) -> Status {
        Status::Connected {
            spoke_id: self.spoke_id.clone(),
            tunnel_ip: self.tunnel_ip,
            connected_at: self.connected_at,
        }
    }
}

struct Current<A> {
    saved: Saved,
    tunnel: Option<Up<A>>,
}

/// Connects, disconnects and switches. Requests only record the target location and answer at
/// once; one reconciling task at a time then moves the session and tunnel towards it, checking
/// the target again between steps.
pub struct Vpn<A, T: Tunnel> {
    api: A,
    tunnel: Arc<T>,
    store: SessionStore,
    /// Every status change happens under this lock, so a published status never contradicts the
    /// latest request.
    target: Mutex<Option<Target>>,
    status: watch::Sender<Status>,
    current: tokio::sync::Mutex<Current<T::Active>>,
    tunnel_recovered: OnceCell<()>,
    /// Notified when a session joins the pending deletions.
    deletions_owed: Notify,
}

impl<A: SessionApi, T: Tunnel> Vpn<A, T> {
    /// Starts recovering from the previous helper in the background; requests are served
    /// meanwhile.
    pub fn new(api: A, tunnel: T, data_dir: &Path) -> Arc<Self> {
        let store = SessionStore::new(data_dir);
        let (saved, error) = recovery::restore(&store);

        let vpn = Arc::new(Self {
            api,
            tunnel: Arc::new(tunnel),
            store,
            target: Mutex::new(None),
            status: watch::Sender::new(Status::Disconnected {
                last_spoke_id: saved.last_spoke_id.clone(),
                error,
            }),
            current: tokio::sync::Mutex::new(Current {
                saved,
                tunnel: None,
            }),
            tunnel_recovered: OnceCell::new(),
            deletions_owed: Notify::new(),
        });
        tokio::spawn(vpn.clone().recover());

        vpn
    }

    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
    }

    /// Switches when already connected elsewhere.
    pub fn connect(self: &Arc<Self>, spoke_id: String, credentials: Credentials) -> Status {
        let status = Status::Connecting {
            spoke_id: spoke_id.clone(),
        };
        {
            let mut target = self.target();
            *target = Some(Target {
                spoke_id,
                credentials,
            });
            self.status.send_replace(status.clone());
        }

        tokio::spawn(self.clone().reconcile());
        status
    }

    pub fn disconnect(self: &Arc<Self>) -> Status {
        let status = {
            let mut target = self.target();
            *target = None;
            self.status.send_if_modified(|status| match status {
                Status::Connecting { spoke_id } | Status::Connected { spoke_id, .. } => {
                    *status = Status::Disconnecting {
                        spoke_id: mem::take(spoke_id),
                    };
                    true
                }
                Status::Disconnected { .. } | Status::Disconnecting { .. } => false,
            });
            self.status.borrow().clone()
        };

        tokio::spawn(self.clone().reconcile());
        status
    }

    fn target(&self) -> MutexGuard<'_, Option<Target>> {
        self.target.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn targets(&self, spoke_id: &str) -> bool {
        self.target()
            .as_ref()
            .is_some_and(|target| target.spoke_id == spoke_id)
    }

    async fn reconcile(self: Arc<Self>) {
        let mut current = self.current.lock().await;
        let mut teardown_error = None;

        loop {
            let target = self.target().clone();

            if let (Some(up), Some(target)) = (&current.tunnel, &target)
                && up.spoke_id == target.spoke_id
            {
                self.publish_for(&target.spoke_id, up.status());
                return;
            }

            if current.tunnel.is_some() || current.saved.session.is_some() {
                if let Err(error) = self.tear_down(&mut current).await {
                    let message = describe(&error);
                    log::warn!("disconnecting failed: {message}");
                    teardown_error = Some(message);
                }
                continue;
            }

            let Some(target) = target else {
                self.settle(&current, teardown_error);
                return;
            };

            if let Err(error) = self.bring_up(&mut current, &target).await {
                let message = describe(&error);
                log::warn!("connecting to {} failed: {message}", target.spoke_id);
                if let Err(error) = self.tear_down(&mut current).await {
                    log::warn!(
                        "cleaning up after the failed connect failed: {}",
                        describe(&error)
                    );
                }
                self.fail(&target.spoke_id, message);
            }
        }
    }

    async fn bring_up(
        &self,
        current: &mut Current<T::Active>,
        target: &Target,
    ) -> Result<(), StepError<T::Error>> {
        self.recover_tunnel().await;
        current.saved.last_spoke_id = Some(target.spoke_id.clone());
        self.store.save(&current.saved)?;

        let private_key = PrivateKey::generate()?;
        let response = self
            .api
            .open(
                &target.credentials,
                &target.spoke_id,
                &private_key.public_key(),
            )
            .await?;
        let preshared_key = response
            .peer
            .preshared_key
            .parse()
            .map_err(ConfigError::PresharedKey)?;
        current.saved.session = Some(Session {
            spoke_id: target.spoke_id.clone(),
            connection_id: response.connection_id.clone(),
            preshared_key,
        });
        self.store.save(&current.saved)?;

        let config = TunnelConfig::from_response(&response, private_key)?;
        if !self.targets(&target.spoke_id) {
            return Ok(());
        }

        let tunnel = self.tunnel.clone();
        let (active, address) = tokio::task::spawn_blocking(move || {
            let endpoint = config.peer.endpoint.resolve()?;
            let plan = AdapterPlan::new(config, endpoint);
            let active = tunnel.up(&plan).map_err(StepError::Up)?;

            Ok::<_, StepError<T::Error>>((active, plan.address.addr()))
        })
        .await??;

        let up = Up {
            active,
            spoke_id: target.spoke_id.clone(),
            tunnel_ip: IpAddr::V4(address),
            connected_at: unix_now(),
        };
        self.publish_for(&target.spoke_id, up.status());
        current.tunnel = Some(up);

        Ok(())
    }

    /// Takes the tunnel down, then deletes the session. A session that failed to delete moves to
    /// the pending deletions for recovery to retry, so a refusing server cannot wedge this loop.
    async fn tear_down(&self, current: &mut Current<T::Active>) -> Result<(), StepError<T::Error>> {
        let down = match current.tunnel.take() {
            Some(up) => {
                let tunnel = self.tunnel.clone();
                tokio::task::spawn_blocking(move || tunnel.down(up.active))
                    .await
                    .map_err(StepError::from)
                    .and_then(|down| down.map_err(StepError::Down))
            }
            None => Ok(()),
        };

        let Some(session) = current.saved.session.take() else {
            return down;
        };
        let closed = self.api.close(&session).await;
        if closed.is_err() {
            current.saved.pending_deletions.push(session);
            self.deletions_owed.notify_one();
        }
        let saved = self.store.save(&current.saved);

        down?;
        closed?;
        saved?;
        Ok(())
    }

    fn publish_for(&self, spoke_id: &str, status: Status) {
        let target = self.target();
        if target
            .as_ref()
            .is_some_and(|target| target.spoke_id == spoke_id)
        {
            self.status.send_replace(status);
        }
    }

    /// Gives up on `spoke_id` unless the user has asked for another location meanwhile.
    fn fail(&self, spoke_id: &str, error: String) {
        let mut target = self.target();
        if target
            .as_ref()
            .is_some_and(|target| target.spoke_id == spoke_id)
        {
            *target = None;
            self.status.send_replace(Status::Disconnected {
                last_spoke_id: Some(spoke_id.to_owned()),
                error: Some(error),
            });
        }
    }

    fn settle(&self, current: &Current<T::Active>, error: Option<String>) {
        let target = self.target();
        if target.is_some() {
            return;
        }

        self.status.send_if_modified(|status| {
            if matches!(status, Status::Disconnected { .. }) {
                return false;
            }

            *status = Status::Disconnected {
                last_spoke_id: current.saved.last_spoke_id.clone(),
                error,
            };
            true
        });
    }
}

/// Only shown to the user, so a clock set before 1970 just shows 0.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
pub mod fakes {
    use std::net::Ipv4Addr;

    use serde_json::json;
    use tokio::sync::Semaphore;

    use super::*;

    pub const TUNNEL_IP: Ipv4Addr = Ipv4Addr::new(10, 21, 0, 7);

    /// A VyprVPN that hands out numbered sessions.
    pub struct FakeApi {
        /// Notified when an `open` starts.
        pub opening: Notify,
        /// Each `open` takes one, so a test can act while a connect is in flight.
        pub permits: Semaphore,
        pub refuse: bool,
        /// How many `close` calls fail before they start succeeding.
        pub refused_closes: Mutex<usize>,
        /// Answers every `close` with 404, like VyprVPN for a session it no longer has.
        pub unknown_sessions: bool,
        pub opened: Mutex<Vec<String>>,
        pub closed: Mutex<Vec<String>>,
    }

    impl Default for FakeApi {
        fn default() -> Self {
            Self {
                opening: Notify::new(),
                permits: Semaphore::new(Semaphore::MAX_PERMITS),
                refuse: false,
                refused_closes: Mutex::new(0),
                unknown_sessions: false,
                opened: Mutex::default(),
                closed: Mutex::default(),
            }
        }
    }

    impl SessionApi for Arc<FakeApi> {
        async fn open(
            &self,
            _credentials: &Credentials,
            spoke_id: &str,
            _client_key: &PublicKey,
        ) -> Result<ConnectResponse, vypr_api::Error> {
            self.opening.notify_one();
            self.permits.acquire().await.unwrap().forget();
            if self.refuse {
                return Err(vypr_api::Error::InvalidCredentials);
            }

            let mut opened = self.opened.lock().unwrap();
            opened.push(spoke_id.to_owned());
            let response = json!({
                "ConnectionId": format!("{spoke_id}-{}", opened.len()),
                "Interface": {"Address": format!("{TUNNEL_IP}/32"), "DNS": ["10.21.0.1"]},
                "Peer": {
                    "PublicKey": "hSDwCYkwp1R0i33ctD73Wg2/Og0mOBr066SpjqqbTmo=",
                    "PresharedKey": "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=",
                    "Endpoint": "203.0.113.10:51820",
                },
            });

            Ok(serde_json::from_value(response).unwrap())
        }

        async fn close(&self, session: &Session) -> Result<(), vypr_api::Error> {
            let mut refused_closes = self.refused_closes.lock().unwrap();
            if *refused_closes > 0 {
                *refused_closes -= 1;
                return Err(vypr_api::Error::InvalidCredentials);
            }
            drop(refused_closes);
            if self.unknown_sessions {
                return Err(vypr_api::Error::Http {
                    path: format!("/vyprvpn/spokes/{}/wg/connections", session.spoke_id),
                    status: vypr_api::StatusCode::NOT_FOUND,
                    body: String::new(),
                });
            }

            self.closed
                .lock()
                .unwrap()
                .push(session.connection_id.0.clone());

            Ok(())
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("the fake adapter refused")]
    pub struct FakeTunnelError;

    /// Numbers the tunnels it brings up and records which are still up.
    #[derive(Default)]
    pub struct FakeTunnel {
        pub refuse: bool,
        pub recoveries: Mutex<usize>,
        pub recovered: Notify,
        pub ups: Mutex<usize>,
        pub live: Mutex<Vec<usize>>,
    }

    impl Tunnel for Arc<FakeTunnel> {
        type Active = usize;
        type Error = FakeTunnelError;

        fn recover(&self) -> Result<(), FakeTunnelError> {
            *self.recoveries.lock().unwrap() += 1;
            self.recovered.notify_one();

            Ok(())
        }

        fn up(&self, plan: &AdapterPlan) -> Result<usize, FakeTunnelError> {
            assert_eq!(*self.recoveries.lock().unwrap(), 1, "up before recovery");
            assert_eq!(plan.address.addr(), TUNNEL_IP);
            if self.refuse {
                return Err(FakeTunnelError);
            }

            let mut ups = self.ups.lock().unwrap();
            *ups += 1;
            self.live.lock().unwrap().push(*ups);

            Ok(*ups)
        }

        fn down(&self, active: usize) -> Result<(), FakeTunnelError> {
            self.live.lock().unwrap().retain(|&live| live != active);

            Ok(())
        }
    }

    pub struct Harness {
        pub api: Arc<FakeApi>,
        pub tunnel: Arc<FakeTunnel>,
        pub data_dir: tempfile::TempDir,
        pub vpn: Arc<Vpn<Arc<FakeApi>, Arc<FakeTunnel>>>,
        pub status: watch::Receiver<Status>,
    }

    impl Harness {
        pub fn new(api: FakeApi, tunnel: FakeTunnel) -> Self {
            Self::in_dir(api, tunnel, tempfile::tempdir().unwrap())
        }

        /// A helper starting on what a previous one left in `data_dir`.
        pub fn in_dir(api: FakeApi, tunnel: FakeTunnel, data_dir: tempfile::TempDir) -> Self {
            let api = Arc::new(api);
            let tunnel = Arc::new(tunnel);
            let vpn = Vpn::new(api.clone(), tunnel.clone(), data_dir.path());
            let status = vpn.subscribe();

            Self {
                api,
                tunnel,
                data_dir,
                vpn,
                status,
            }
        }

        pub fn connect(&self, spoke_id: &str) -> Status {
            self.vpn.connect(
                spoke_id.to_owned(),
                Credentials {
                    username: "alice".into(),
                    password: "correct horse".into(),
                },
            )
        }

        pub async fn settled(&mut self) -> Status {
            self.status
                .wait_for(|status| {
                    matches!(
                        status,
                        Status::Connected { .. } | Status::Disconnected { .. }
                    )
                })
                .await
                .unwrap()
                .clone()
        }

        pub fn opened(&self) -> Vec<String> {
            self.api.opened.lock().unwrap().clone()
        }

        pub fn closed(&self) -> Vec<String> {
            self.api.closed.lock().unwrap().clone()
        }

        pub fn live_tunnels(&self) -> usize {
            self.tunnel.live.lock().unwrap().len()
        }

        pub fn saved(&self) -> Saved {
            SessionStore::new(self.data_dir.path()).load().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fakes::{FakeApi, FakeTunnel, FakeTunnelError, Harness, TUNNEL_IP};
    use super::*;

    fn connected_to(spoke_id: &str) -> impl Fn(&Status) -> bool {
        move |status| {
            matches!(
                status,
                Status::Connected { spoke_id: connected, tunnel_ip, connected_at }
                    if connected == spoke_id && *tunnel_ip == IpAddr::V4(TUNNEL_IP) && *connected_at > 0
            )
        }
    }

    fn disconnected(last_spoke_id: &str, error: Option<&str>) -> Status {
        Status::Disconnected {
            last_spoke_id: Some(last_spoke_id.into()),
            error: error.map(Into::into),
        }
    }

    #[tokio::test]
    async fn connects_then_disconnects() {
        let mut vpn = Harness::new(FakeApi::default(), FakeTunnel::default());

        assert_eq!(
            vpn.connect("de1"),
            Status::Connecting {
                spoke_id: "de1".into()
            }
        );
        let status = vpn.settled().await;

        assert!(connected_to("de1")(&status), "{status:?}");
        assert_eq!(vpn.opened(), ["de1"]);
        assert_eq!(vpn.live_tunnels(), 1);
        let saved = vpn.saved();
        assert_eq!(saved.last_spoke_id.as_deref(), Some("de1"));
        assert_eq!(saved.session.unwrap().connection_id.0, "de1-1");

        assert_eq!(
            vpn.vpn.disconnect(),
            Status::Disconnecting {
                spoke_id: "de1".into()
            }
        );

        assert_eq!(vpn.settled().await, disconnected("de1", None));
        assert_eq!(vpn.closed(), ["de1-1"]);
        assert_eq!(vpn.live_tunnels(), 0);
        assert_eq!(
            vpn.saved(),
            Saved {
                last_spoke_id: Some("de1".into()),
                ..Saved::default()
            }
        );
    }

    #[tokio::test]
    async fn reports_an_api_failure() {
        let mut vpn = Harness::new(
            FakeApi {
                refuse: true,
                ..FakeApi::default()
            },
            FakeTunnel::default(),
        );

        vpn.connect("de1");

        assert_eq!(
            vpn.settled().await,
            disconnected("de1", Some("invalid username or password"))
        );
        assert!(vpn.closed().is_empty());
        assert_eq!(*vpn.tunnel.ups.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn deletes_the_session_when_the_tunnel_fails() {
        let mut vpn = Harness::new(
            FakeApi::default(),
            FakeTunnel {
                refuse: true,
                ..FakeTunnel::default()
            },
        );

        vpn.connect("de1");

        assert_eq!(
            vpn.settled().await,
            disconnected(
                "de1",
                Some("bringing the tunnel up failed: the fake adapter refused")
            )
        );
        assert_eq!(vpn.closed(), ["de1-1"]);
        assert_eq!(vpn.saved().session, None);
    }

    #[tokio::test]
    async fn switching_leaves_one_session_and_one_tunnel() {
        let mut vpn = Harness::new(FakeApi::default(), FakeTunnel::default());
        vpn.connect("de1");
        vpn.settled().await;

        vpn.connect("uk1");
        let status = vpn.settled().await;

        assert!(connected_to("uk1")(&status), "{status:?}");
        assert_eq!(vpn.opened(), ["de1", "uk1"]);
        assert_eq!(vpn.closed(), ["de1-1"]);
        assert_eq!(*vpn.tunnel.live.lock().unwrap(), [2]);
        assert_eq!(vpn.saved().session.unwrap().connection_id.0, "uk1-2");
    }

    #[tokio::test]
    async fn disconnecting_while_connecting_deletes_the_new_session() {
        let mut vpn = Harness::new(
            FakeApi {
                permits: tokio::sync::Semaphore::new(0),
                ..FakeApi::default()
            },
            FakeTunnel::default(),
        );
        vpn.connect("de1");
        vpn.api.opening.notified().await;

        assert_eq!(
            vpn.vpn.disconnect(),
            Status::Disconnecting {
                spoke_id: "de1".into()
            }
        );
        vpn.api.permits.add_permits(1);

        assert_eq!(vpn.settled().await, disconnected("de1", None));
        assert_eq!(vpn.opened(), ["de1"]);
        assert_eq!(vpn.closed(), ["de1-1"]);
        assert_eq!(*vpn.tunnel.ups.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn keeps_a_session_that_failed_to_delete_for_recovery() {
        let mut vpn = Harness::new(
            FakeApi {
                refused_closes: Mutex::new(usize::MAX),
                ..FakeApi::default()
            },
            FakeTunnel::default(),
        );
        vpn.connect("de1");
        vpn.settled().await;

        vpn.vpn.disconnect();

        assert_eq!(
            vpn.settled().await,
            disconnected("de1", Some("invalid username or password"))
        );
        assert_eq!(vpn.live_tunnels(), 0);
        let saved = vpn.saved();
        assert_eq!(saved.session, None);
        assert_eq!(
            saved
                .pending_deletions
                .iter()
                .map(|session| session.connection_id.0.as_str())
                .collect::<Vec<_>>(),
            ["de1-1"]
        );
    }

    #[tokio::test]
    async fn disconnecting_while_disconnected_changes_nothing() {
        let vpn = Harness::new(FakeApi::default(), FakeTunnel::default());

        assert_eq!(
            vpn.vpn.disconnect(),
            Status::Disconnected {
                last_spoke_id: None,
                error: None
            }
        );
    }

    #[tokio::test]
    async fn remembers_the_last_location_across_restarts() {
        let mut vpn = Harness::new(FakeApi::default(), FakeTunnel::default());
        vpn.connect("de1");
        vpn.settled().await;
        vpn.vpn.disconnect();
        vpn.settled().await;

        let restarted = Vpn::new(vpn.api.clone(), vpn.tunnel.clone(), vpn.data_dir.path());

        assert_eq!(*restarted.subscribe().borrow(), disconnected("de1", None));
    }

    #[test]
    fn describes_the_causes_once() {
        let error = StepError::<FakeTunnelError>::Endpoint(EndpointError::Resolve {
            endpoint: "nowhere.invalid:51820".parse().unwrap(),
            source: std::io::Error::other("no such host"),
        });

        assert_eq!(
            describe(&error),
            "resolving endpoint nowhere.invalid:51820: no such host"
        );
    }
}
