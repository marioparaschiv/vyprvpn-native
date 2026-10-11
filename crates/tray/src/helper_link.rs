use std::collections::HashMap;
use std::future;
use std::io;
use std::thread;
use std::time::Duration;

use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::traits::StreamCommon as _;
use interprocess::local_socket::traits::tokio::Stream as _;
use interprocess::local_socket::{GenericFilePath, ToFsName};
use ipc::{Account, Location, Method, Outcome, Reply, Request, Response, ServerMessage, Status};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, WriteHalf};
use tokio::sync::{mpsc, oneshot, watch};

use crate::service;
use crate::trust::{self, Untrusted};

const MIN_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(2);
const MIN_LOCATIONS_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_LOCATIONS_RETRY_DELAY: Duration = Duration::from_secs(30);

/// How connecting fails while nothing serves the helper's endpoint, so the helper needs starting.
#[cfg(windows)]
const NOT_SERVED: &[io::ErrorKind] = &[io::ErrorKind::NotFound];
/// A macOS helper that exited or crashed can leave its socket file behind, which refuses connections.
#[cfg(target_os = "macos")]
const NOT_SERVED: &[io::ErrorKind] = &[io::ErrorKind::NotFound, io::ErrorKind::ConnectionRefused];

/// Where the helper listens, and whether the tray app starts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Helper {
    /// The installed service, started through the Service Control Manager whenever it is not running.
    Service,
    /// A helper started by hand with `vyprlite-helper --console`, running as `user_sid`.
    #[cfg(windows)]
    Console { pipe_name: String, user_sid: String },
    /// A helper started by hand with `vyprlite-helper --console`, running as this user.
    #[cfg(target_os = "macos")]
    Console { pipe_name: String },
}

impl Helper {
    fn pipe_name(&self) -> &str {
        match self {
            #[cfg(windows)]
            Self::Service => ipc::SERVICE_PIPE_NAME,
            #[cfg(target_os = "macos")]
            Self::Service => ipc::DAEMON_SOCKET_PATH,
            Self::Console { pipe_name, .. } => pipe_name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperState {
    NotRunning,
    /// The helper is not running and the Service Control Manager refused to start it, for this reason.
    StartFailed(String),
    /// Something other than the helper serves its pipe; nothing is sent to it.
    Untrusted(Untrusted),
    Running(Status),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Locations {
    Loading,
    /// Fetching failed for this reason; the link retries.
    Failed(String),
    Loaded(Vec<Location>),
}

/// Everything the tray app shows about the helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkState {
    pub helper: HelperState,
    /// `None` until a running helper has said.
    pub account: Option<Account>,
    /// Kept across helper restarts once loaded.
    pub locations: Locations,
    /// The location the user picked; `None` until they pick one.
    pub selected: Option<String>,
}

impl LinkState {
    /// Where Connect goes: the location the user picked, else the one in use or last used, else
    /// the first one listed.
    pub fn selected_spoke_id(&self) -> Option<&str> {
        let current = match &self.helper {
            HelperState::Running(Status::Disconnected { last_spoke_id, .. }) => {
                last_spoke_id.as_deref()
            }
            HelperState::Running(
                Status::Connecting { spoke_id }
                | Status::Connected { spoke_id, .. }
                | Status::Disconnecting { spoke_id },
            ) => Some(spoke_id.as_str()),
            HelperState::NotRunning | HelperState::StartFailed(_) | HelperState::Untrusted(_) => {
                None
            }
        };
        let first = match &self.locations {
            Locations::Loaded(locations) => locations.first().map(|first| first.spoke_id.as_str()),
            Locations::Loading | Locations::Failed(_) => None,
        };

        self.selected.as_deref().or(current).or(first)
    }
}

impl Default for LinkState {
    fn default() -> Self {
        Self {
            helper: HelperState::NotRunning,
            account: None,
            locations: Locations::Loading,
            selected: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    #[error("the helper is not running")]
    HelperNotRunning,
    #[error("{0}")]
    Refused(String),
}

/// Sends requests on behalf of the UI, such as logging in.
#[derive(Debug, Clone)]
pub struct Requests {
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Sender<LinkState>,
}

#[derive(Debug)]
pub struct Command {
    pub method: Method,
    done: oneshot::Sender<Result<(), RequestError>>,
}

impl Command {
    pub fn finish(self, result: Result<(), RequestError>) {
        // Nobody waits any more when the window that asked has closed.
        self.done.send(result).ok();
    }
}

impl Requests {
    /// Requests that go to the returned receiver rather than to a helper.
    #[cfg(test)]
    pub fn intercepted() -> (Self, mpsc::UnboundedReceiver<Command>) {
        let (requests, commands) = mpsc::unbounded_channel();

        (
            Self {
                commands: requests,
                state: watch::Sender::new(LinkState::default()),
            },
            commands,
        )
    }

    #[cfg(test)]
    pub fn observe(&self) -> watch::Receiver<LinkState> {
        self.state.subscribe()
    }

    /// Resolves once the helper has carried out `method`.
    pub async fn send(&self, method: Method) -> Result<(), RequestError> {
        let (done, finished) = oneshot::channel();
        self.commands
            .send(Command { method, done })
            .map_err(|_| RequestError::HelperNotRunning)?;

        finished
            .await
            .unwrap_or(Err(RequestError::HelperNotRunning))
    }

    /// Connects to `spoke_id`, switching location if already connected.
    pub async fn connect(&self, spoke_id: String) -> Result<(), RequestError> {
        self.send(Method::Connect { spoke_id }).await
    }

    pub async fn disconnect(&self) -> Result<(), RequestError> {
        self.send(Method::Disconnect).await
    }

    /// Picks where Connect goes, without connecting.
    pub fn select(&self, spoke_id: String) {
        update(&self.state, |state| state.selected = Some(spoke_id));
    }
}

#[derive(Debug, thiserror::Error)]
enum LinkError {
    #[error("pipe I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("malformed message: {0}")]
    Codec(#[from] ipc::CodecError),
    #[error("helper refused a request: {0}")]
    Refused(String),
    #[error("helper answered request {0}, which was never sent")]
    UnexpectedResponse(u64),
}

/// Runs [`run`] on its own thread so pipe I/O never blocks the UI.
pub fn spawn(helper: Helper) -> io::Result<(watch::Receiver<LinkState>, Requests)> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (state, observed) = watch::channel(LinkState::default());
    let (requests, mut commands) = mpsc::unbounded_channel();
    let requests = Requests {
        commands: requests,
        state: state.clone(),
    };

    thread::Builder::new()
        .name("helper-link".into())
        .spawn(move || runtime.block_on(run(&helper, &state, &mut commands)))?;

    Ok((observed, requests))
}

/// Keeps `state` in sync with `helper`, starting it if needed and reconnecting whenever it goes away.
async fn run(
    helper: &Helper,
    state: &watch::Sender<LinkState>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) {
    let mut retry_delay = MIN_RETRY_DELAY;

    loop {
        let unreachable = match connect(helper.pipe_name()).await {
            Ok(stream) => match verify(helper, &stream) {
                Ok(()) => {
                    retry_delay = MIN_RETRY_DELAY;
                    if let Err(error) = follow(stream, state, commands).await {
                        log::warn!("helper connection lost: {error}");
                    }
                    HelperState::NotRunning
                }
                Err(untrusted) => HelperState::Untrusted(untrusted),
            },
            Err(error) if NOT_SERVED.contains(&error.kind()) => match helper {
                Helper::Service => match service::start() {
                    Ok(()) => HelperState::NotRunning,
                    Err(error) => HelperState::StartFailed(error.to_string()),
                },
                Helper::Console { .. } => HelperState::NotRunning,
            },
            Err(error) => {
                log::warn!("connecting to the helper failed: {error}");
                HelperState::NotRunning
            }
        };

        let changed = update(state, |state| {
            state.helper = unreachable.clone();
            state.account = None;
        });
        if changed {
            match unreachable {
                HelperState::StartFailed(reason) => log::warn!("{reason}"),
                HelperState::Untrusted(untrusted) => log::error!("{untrusted}"),
                HelperState::NotRunning | HelperState::Running(_) => {}
            }
        }

        let retry = tokio::time::sleep(retry_delay);
        tokio::pin!(retry);
        loop {
            tokio::select! {
                () = &mut retry => break,
                Some(command) = commands.recv() => command.finish(Err(RequestError::HelperNotRunning)),
            }
        }
        retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
    }
}

/// Checks who serves the pipe before anything is sent over it.
fn verify(helper: &Helper, stream: &Stream) -> Result<(), Untrusted> {
    let peer = stream
        .peer_creds()
        .map_err(|error| Untrusted::Unverifiable(error.to_string()))?;

    trust::verify_peer(helper, &peer)
}

async fn connect(pipe_name: &str) -> io::Result<Stream> {
    Stream::connect(pipe_name.to_fs_name::<GenericFilePath>()?).await
}

/// What to do with the answer to a request.
enum Pending {
    /// Show it; the connection is useless if it fails.
    Query,
    /// Show it, or the error, and retry until it succeeds.
    Locations,
    Command(Command),
}

struct Connection {
    writer: WriteHalf<Stream>,
    next_id: u64,
    pending: HashMap<u64, Pending>,
}

impl Connection {
    async fn request(&mut self, method: Method, pending: Pending) -> Result<(), LinkError> {
        self.next_id += 1;
        let request = Request {
            id: self.next_id,
            method,
        };
        self.writer.write_all(&ipc::encode(&request)?).await?;
        self.pending.insert(request.id, pending);

        Ok(())
    }
}

async fn follow(
    stream: Stream,
    state: &watch::Sender<LinkState>,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> Result<(), LinkError> {
    let (reader, writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let mut connection = Connection {
        writer,
        next_id: 0,
        pending: HashMap::new(),
    };
    let mut locations_retry_delay = MIN_LOCATIONS_RETRY_DELAY;
    let mut locations_retry = None;

    connection.request(Method::Status, Pending::Query).await?;
    connection.request(Method::Account, Pending::Query).await?;
    if !matches!(state.borrow().locations, Locations::Loaded(_)) {
        connection
            .request(Method::Locations, Pending::Locations)
            .await?;
    }

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else {
                    return Ok(());
                };

                match ipc::decode(&line)? {
                    ServerMessage::Status(status) => show(state, Reply::Status(status)),
                    ServerMessage::Account(account) => show(state, Reply::Account(account)),
                    ServerMessage::Response(Response { id, outcome }) => {
                        let pending = connection
                            .pending
                            .remove(&id)
                            .ok_or(LinkError::UnexpectedResponse(id))?;

                        match (pending, outcome) {
                            (Pending::Command(command), Outcome::Result(reply)) => {
                                show(state, reply);
                                command.finish(Ok(()));
                            }
                            (_, Outcome::Result(reply)) => show(state, reply),
                            (Pending::Query, Outcome::Error(message)) => {
                                return Err(LinkError::Refused(message));
                            }
                            (Pending::Locations, Outcome::Error(message)) => {
                                log::warn!("{message}; retrying in {locations_retry_delay:?}");
                                update(state, |state| state.locations = Locations::Failed(message));
                                locations_retry = Some(Box::pin(tokio::time::sleep(locations_retry_delay)));
                                locations_retry_delay = (locations_retry_delay * 2).min(MAX_LOCATIONS_RETRY_DELAY);
                            }
                            (Pending::Command(command), Outcome::Error(message)) => {
                                command.finish(Err(RequestError::Refused(message)));
                            }
                        }
                    }
                }
            }
            Some(command) = commands.recv() => {
                connection.request(command.method.clone(), Pending::Command(command)).await?;
            }
            () = async {
                match locations_retry.as_mut() {
                    Some(retry) => retry.await,
                    None => future::pending().await,
                }
            } => {
                locations_retry = None;
                connection.request(Method::Locations, Pending::Locations).await?;
            }
        }
    }
}

fn show(state: &watch::Sender<LinkState>, reply: Reply) {
    update(state, |state| match reply {
        Reply::Status(status) => state.helper = HelperState::Running(status),
        Reply::Account(account) => state.account = Some(account),
        Reply::Locations(locations) => state.locations = Locations::Loaded(locations),
    });
}

/// Applies `change`, notifying observers only if it changed anything. Returns whether it did.
fn update(state: &watch::Sender<LinkState>, change: impl FnOnce(&mut LinkState)) -> bool {
    state.send_if_modified(|current| {
        let before = current.clone();
        change(current);

        *current != before
    })
}

#[cfg(test)]
mod tests {
    use interprocess::local_socket::ListenerOptions;
    use interprocess::local_socket::tokio::Listener;
    use interprocess::local_socket::traits::tokio::Listener as _;
    use tokio::io::ReadHalf;

    use super::*;

    fn start_fake_helper(pipe_name: &str) -> Listener {
        ListenerOptions::new()
            .name(pipe_name.to_fs_name::<GenericFilePath>().unwrap())
            .create_tokio()
            .unwrap()
    }

    struct TrayAppConnection {
        lines: tokio::io::Lines<BufReader<ReadHalf<Stream>>>,
        writer: WriteHalf<Stream>,
    }

    impl TrayAppConnection {
        async fn accept(helper: &Listener) -> Self {
            let (reader, writer) = tokio::io::split(helper.accept().await.unwrap());

            Self {
                lines: BufReader::new(reader).lines(),
                writer,
            }
        }

        /// Answers the next request, which must be for `method`.
        async fn answer(&mut self, method: Method, outcome: Outcome) {
            let line = self.lines.next_line().await.unwrap().unwrap();
            let request: Request = ipc::decode(&line).unwrap();
            assert_eq!(request.method, method);

            self.send(&ServerMessage::Response(Response {
                id: request.id,
                outcome,
            }))
            .await;
        }

        async fn send(&mut self, message: &ServerMessage) {
            self.writer
                .write_all(&ipc::encode(message).unwrap())
                .await
                .unwrap();
        }

        async fn greet(&mut self, locations: Option<Outcome>) {
            self.answer(
                Method::Status,
                Outcome::Result(Reply::Status(Status::Disconnected {
                    last_spoke_id: None,
                    error: None,
                })),
            )
            .await;
            self.answer(
                Method::Account,
                Outcome::Result(Reply::Account(Account::LoggedOut)),
            )
            .await;
            if let Some(locations) = locations {
                self.answer(Method::Locations, locations).await;
            }
        }
    }

    fn perth() -> Location {
        Location {
            name: "Australia - Perth".into(),
            country: "Australia".into(),
            country_code: "AUS".into(),
            region: "Oceania".into(),
            spoke_id: "au3".into(),
            hostname: "au3.vpn.goldenfrog.com".into(),
        }
    }

    fn loaded() -> Outcome {
        Outcome::Result(Reply::Locations(vec![perth()]))
    }

    #[cfg(windows)]
    fn pipe_name(test: &str) -> String {
        format!(r"\\.\pipe\vyprlite-tray-test-{test}-{}", std::process::id())
    }

    #[cfg(target_os = "macos")]
    fn pipe_name(test: &str) -> String {
        std::env::temp_dir()
            .join(format!(
                "vyprlite-tray-test-{test}-{}.sock",
                std::process::id()
            ))
            .into_os_string()
            .into_string()
            .unwrap()
    }

    struct Link {
        observed: watch::Receiver<LinkState>,
        requests: Requests,
        task: tokio::task::JoinHandle<()>,
    }

    impl Link {
        #[cfg(windows)]
        fn start(pipe_name: &str) -> Self {
            Self::start_expecting(pipe_name, &ipc::current_user_sid().unwrap())
        }

        #[cfg(windows)]
        fn start_expecting(pipe_name: &str, user_sid: &str) -> Self {
            Self::follow(Helper::Console {
                pipe_name: pipe_name.to_owned(),
                user_sid: user_sid.to_owned(),
            })
        }

        #[cfg(target_os = "macos")]
        fn start(pipe_name: &str) -> Self {
            Self::follow(Helper::Console {
                pipe_name: pipe_name.to_owned(),
            })
        }

        fn follow(helper: Helper) -> Self {
            let (state, observed) = watch::channel(LinkState::default());
            let (requests, mut commands) = mpsc::unbounded_channel();
            let requests = Requests {
                commands: requests,
                state: state.clone(),
            };
            let task = tokio::spawn(async move { run(&helper, &state, &mut commands).await });

            Self {
                observed,
                requests,
                task,
            }
        }

        async fn wait_for(&mut self, done: impl FnMut(&LinkState) -> bool) -> LinkState {
            self.observed.wait_for(done).await.unwrap().clone()
        }
    }

    impl Drop for Link {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn follows_the_helper_across_restarts() {
        let pipe_name = pipe_name("restarts");
        let fake_helper = start_fake_helper(&pipe_name);
        let mut link = Link::start(&pipe_name);

        let mut connection = TrayAppConnection::accept(&fake_helper).await;
        connection.greet(Some(loaded())).await;
        assert_eq!(
            link.wait_for(|state| matches!(state.locations, Locations::Loaded(_)))
                .await,
            LinkState {
                helper: HelperState::Running(Status::Disconnected {
                    last_spoke_id: None,
                    error: None,
                }),
                account: Some(Account::LoggedOut),
                locations: Locations::Loaded(vec![perth()]),
                selected: None,
            }
        );

        drop((connection, fake_helper));
        assert_eq!(
            link.wait_for(|state| state.helper == HelperState::NotRunning)
                .await,
            LinkState {
                helper: HelperState::NotRunning,
                account: None,
                locations: Locations::Loaded(vec![perth()]),
                selected: None,
            }
        );

        let restarted = start_fake_helper(&pipe_name);
        let mut connection = TrayAppConnection::accept(&restarted).await;
        connection.greet(None).await;
        link.wait_for(|state| state.account.is_some()).await;
    }

    #[tokio::test(start_paused = true)]
    async fn retries_the_locations_with_a_backoff() {
        let pipe_name = pipe_name("locations");
        let fake_helper = start_fake_helper(&pipe_name);
        let mut link = Link::start(&pipe_name);
        let mut connection = TrayAppConnection::accept(&fake_helper).await;
        let failed = || Outcome::Error("fetching the locations failed: offline".into());

        connection.greet(Some(failed())).await;
        let state = link
            .wait_for(|state| matches!(state.locations, Locations::Failed(_)))
            .await;
        assert_eq!(
            state.locations,
            Locations::Failed("fetching the locations failed: offline".into())
        );

        let started = tokio::time::Instant::now();
        connection.answer(Method::Locations, failed()).await;
        connection.answer(Method::Locations, loaded()).await;

        link.wait_for(|state| matches!(state.locations, Locations::Loaded(_)))
            .await;
        assert_eq!(started.elapsed(), Duration::from_secs(1 + 2));
    }

    #[tokio::test(start_paused = true)]
    async fn reports_whether_a_login_succeeded() {
        let pipe_name = pipe_name("login");
        let fake_helper = start_fake_helper(&pipe_name);
        let mut link = Link::start(&pipe_name);
        let mut connection = TrayAppConnection::accept(&fake_helper).await;
        connection.greet(Some(loaded())).await;
        link.wait_for(|state| state.account.is_some()).await;
        let alice = ipc::Credentials {
            username: "alice".into(),
            password: "correct horse".into(),
        };

        let (refused, ()) = tokio::join!(
            link.requests.send(Method::Login(alice.clone())),
            connection.answer(
                Method::Login(alice.clone()),
                Outcome::Error("invalid username or password".into())
            )
        );
        assert_eq!(
            refused,
            Err(RequestError::Refused("invalid username or password".into()))
        );

        let logged_in = Account::LoggedIn {
            username: "alice".into(),
        };
        let (accepted, ()) = tokio::join!(
            link.requests.send(Method::Login(alice.clone())),
            connection.answer(
                Method::Login(alice),
                Outcome::Result(Reply::Account(logged_in.clone()))
            )
        );
        assert_eq!(accepted, Ok(()));
        assert_eq!(link.observed.borrow().account, Some(logged_in));
    }

    #[tokio::test(start_paused = true)]
    async fn refuses_requests_while_the_helper_is_down() {
        let link = Link::start(&pipe_name("down"));

        assert_eq!(
            link.requests.send(Method::Logout).await,
            Err(RequestError::HelperNotRunning)
        );
    }

    #[cfg(windows)]
    #[tokio::test(start_paused = true)]
    async fn sends_nothing_to_a_pipe_served_by_another_user() {
        let pipe_name = pipe_name("squatted");
        let squatter = start_fake_helper(&pipe_name);
        let mut link = Link::start_expecting(&pipe_name, "S-1-5-21-1-2-3-1001");

        let mut connection = TrayAppConnection::accept(&squatter).await;
        let state = link
            .wait_for(|state| matches!(state.helper, HelperState::Untrusted(_)))
            .await;
        assert!(
            matches!(
                state.helper,
                HelperState::Untrusted(Untrusted::WrongUser { ref expected, .. })
                    if expected == "S-1-5-21-1-2-3-1001"
            ),
            "{state:?}"
        );

        let (refused, received) = tokio::join!(
            link.requests.send(Method::Logout),
            connection.lines.next_line()
        );
        assert_eq!(refused, Err(RequestError::HelperNotRunning));
        assert_eq!(received.unwrap(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn connects_and_follows_the_pushed_status() {
        let pipe_name = pipe_name("connect");
        let fake_helper = start_fake_helper(&pipe_name);
        let mut link = Link::start(&pipe_name);
        let mut connection = TrayAppConnection::accept(&fake_helper).await;
        connection.greet(Some(loaded())).await;
        link.wait_for(|state| state.account.is_some()).await;
        let connecting = Status::Connecting {
            spoke_id: "au3".into(),
        };

        let (accepted, ()) = tokio::join!(
            link.requests.connect("au3".into()),
            connection.answer(
                Method::Connect {
                    spoke_id: "au3".into()
                },
                Outcome::Result(Reply::Status(connecting.clone()))
            )
        );
        assert_eq!(accepted, Ok(()));
        assert_eq!(
            link.observed.borrow().helper,
            HelperState::Running(connecting)
        );

        let connected = Status::Connected {
            spoke_id: "au3".into(),
            tunnel_ip: [10, 1, 2, 3].into(),
            connected_at: 1_760_140_800,
        };
        connection
            .send(&ServerMessage::Status(connected.clone()))
            .await;
        link.wait_for(|state| state.helper == HelperState::Running(connected.clone()))
            .await;

        let (refused, ()) = tokio::join!(
            link.requests.disconnect(),
            connection.answer(
                Method::Disconnect,
                Outcome::Error("removing the adapter failed".into())
            )
        );
        assert_eq!(
            refused,
            Err(RequestError::Refused("removing the adapter failed".into()))
        );
    }

    #[test]
    fn selects_the_picked_then_the_current_then_the_first_location() {
        let mut state = LinkState {
            locations: Locations::Loaded(vec![perth()]),
            ..LinkState::default()
        };
        assert_eq!(state.selected_spoke_id(), Some("au3"));

        state.helper = HelperState::Running(Status::Disconnected {
            last_spoke_id: Some("de1".into()),
            error: None,
        });
        assert_eq!(state.selected_spoke_id(), Some("de1"));

        state.helper = HelperState::Running(Status::Connected {
            spoke_id: "us5".into(),
            tunnel_ip: [10, 1, 2, 3].into(),
            connected_at: 1_760_140_800,
        });
        assert_eq!(state.selected_spoke_id(), Some("us5"));

        let (requests, _commands) = Requests::intercepted();
        let mut observed = requests.state.subscribe();
        requests.select("au2".into());
        state.selected = observed.borrow_and_update().selected.clone();
        assert_eq!(state.selected_spoke_id(), Some("au2"));

        assert_eq!(LinkState::default().selected_spoke_id(), None);
    }
}
