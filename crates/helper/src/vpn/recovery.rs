use std::sync::Arc;
use std::time::Duration;

use vypr_api::StatusCode;

use super::{SessionApi, Tunnel, Vpn, describe};
use crate::session_store::{Saved, Session, SessionStore};

const FIRST_RETRY: Duration = Duration::from_secs(2);
const MAX_RETRY: Duration = Duration::from_secs(5 * 60);

/// Reads what the previous helper left, before any request is served. Its tunnel died with it,
/// so a recorded session moves to the pending deletions before a connect can overwrite it. A
/// corrupt record is reset, and the error is returned for the status.
pub(super) fn restore(store: &SessionStore) -> (Saved, Option<String>) {
    let mut saved = match store.load() {
        Ok(saved) => saved,
        Err(error) => {
            let message =
                format!("the saved connection was unreadable and has been reset: {error}");
            log::error!("{message}");
            if let Err(error) = store.save(&Saved::default()) {
                log::error!("resetting the saved connection failed: {error}");
            }

            return (Saved::default(), Some(message));
        }
    };

    if let Some(session) = saved.session.take() {
        log::info!(
            "deleting session {} on {} that a previous helper left",
            session.connection_id.0,
            session.spoke_id
        );
        saved.pending_deletions.push(session);
        if let Err(error) = store.save(&saved) {
            log::error!("recording the leftover session for deletion failed: {error}");
        }
    }

    (saved, None)
}

impl<A: SessionApi, T: Tunnel> Vpn<A, T> {
    pub(super) async fn recover(self: Arc<Self>) {
        self.recover_tunnel().await;
        self.delete_pending_sessions().await;
    }

    /// Runs the platform's recovery once; a connect waits for it before bringing a tunnel up.
    pub(super) async fn recover_tunnel(&self) {
        self.tunnel_recovered
            .get_or_init(|| async {
                let tunnel = self.tunnel.clone();
                match tokio::task::spawn_blocking(move || tunnel.recover()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => log::error!("{error}"),
                    Err(error) => log::error!("undoing a leftover tunnel panicked: {error}"),
                }
            })
            .await;
    }

    /// Deletes the pending sessions, retrying failures with a capped backoff, then waits for the
    /// next one. Takes the connection lock only to read and record, so requests never wait on
    /// VyprVPN here.
    async fn delete_pending_sessions(&self) {
        let mut retry_in = FIRST_RETRY;

        loop {
            let pending = self.current.lock().await.saved.pending_deletions.clone();
            if pending.is_empty() {
                self.deletions_owed.notified().await;
                retry_in = FIRST_RETRY;
                continue;
            }

            let mut deleted = Vec::new();
            for session in &pending {
                match self.api.close(session).await {
                    Ok(()) => deleted.push(session.clone()),
                    Err(vypr_api::Error::Http {
                        status: StatusCode::NOT_FOUND,
                        ..
                    }) => {
                        log::info!(
                            "session {} on {} was already gone",
                            session.connection_id.0,
                            session.spoke_id
                        );
                        deleted.push(session.clone());
                    }
                    Err(error) => log::warn!(
                        "deleting session {} on {} failed; retrying in {retry_in:?}: {}",
                        session.connection_id.0,
                        session.spoke_id,
                        describe(&error)
                    ),
                }
            }
            let all_deleted = deleted.len() == pending.len();
            self.forget(&deleted).await;

            if all_deleted {
                retry_in = FIRST_RETRY;
            } else {
                tokio::time::sleep(retry_in).await;
                retry_in = (retry_in * 2).min(MAX_RETRY);
            }
        }
    }

    async fn forget(&self, deleted: &[Session]) {
        if deleted.is_empty() {
            return;
        }

        let mut current = self.current.lock().await;
        current
            .saved
            .pending_deletions
            .retain(|session| !deleted.contains(session));
        if let Err(error) = self.store.save(&current.saved) {
            log::error!("recording the deleted sessions failed: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Mutex;

    use ipc::Status;
    use tunnel::ConnectionId;

    use super::*;
    use crate::vpn::fakes::{FakeApi, FakeTunnel, Harness};

    const PRESHARED: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

    fn session(connection_id: &str) -> Session {
        Session {
            spoke_id: "de1".into(),
            connection_id: ConnectionId(connection_id.into()),
            preshared_key: PRESHARED.parse().unwrap(),
        }
    }

    fn left_behind(saved: &Saved) -> tempfile::TempDir {
        let data_dir = tempfile::tempdir().unwrap();
        SessionStore::new(data_dir.path()).save(saved).unwrap();

        data_dir
    }

    fn disconnected_from_de1() -> Status {
        Status::Disconnected {
            last_spoke_id: Some("de1".into()),
            error: None,
        }
    }

    /// Lets the background recovery run until it waits on the clock or for more work; the clock
    /// is paused, so this does not really sleep.
    async fn let_recovery_run_for(duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    #[tokio::test(start_paused = true)]
    async fn no_session_reports_disconnected_from_the_last_location() {
        let saved = Saved {
            last_spoke_id: Some("de1".into()),
            ..Saved::default()
        };
        let vpn = Harness::in_dir(
            FakeApi::default(),
            FakeTunnel::default(),
            left_behind(&saved),
        );

        let_recovery_run_for(MAX_RETRY).await;

        assert_eq!(*vpn.status.borrow(), disconnected_from_de1());
        assert!(vpn.closed().is_empty());
        assert_eq!(vpn.saved(), saved);
    }

    #[tokio::test(start_paused = true)]
    async fn session_without_tunnel_is_deleted_and_cleared() {
        let vpn = Harness::in_dir(
            FakeApi::default(),
            FakeTunnel::default(),
            left_behind(&Saved {
                last_spoke_id: Some("de1".into()),
                session: Some(session("48213977")),
                pending_deletions: Vec::new(),
            }),
        );

        assert_eq!(*vpn.status.borrow(), disconnected_from_de1());
        assert_eq!(vpn.saved().session, None);

        let_recovery_run_for(FIRST_RETRY).await;

        assert_eq!(vpn.closed(), ["48213977"]);
        assert_eq!(
            vpn.saved(),
            Saved {
                last_spoke_id: Some("de1".into()),
                ..Saved::default()
            }
        );
        assert_eq!(*vpn.status.borrow(), disconnected_from_de1());
    }

    #[tokio::test(start_paused = true)]
    async fn retries_a_failed_delete_with_backoff() {
        let vpn = Harness::in_dir(
            FakeApi {
                refused_closes: Mutex::new(2),
                ..FakeApi::default()
            },
            FakeTunnel::default(),
            left_behind(&Saved {
                last_spoke_id: Some("de1".into()),
                session: Some(session("48213977")),
                pending_deletions: vec![session("48213900")],
            }),
        );

        let_recovery_run_for(FIRST_RETRY / 2).await;
        assert!(vpn.closed().is_empty());
        assert_eq!(vpn.saved().pending_deletions.len(), 2);

        let_recovery_run_for(FIRST_RETRY).await;
        assert_eq!(vpn.closed(), ["48213900", "48213977"]);
        assert_eq!(vpn.saved().pending_deletions, []);
    }

    #[tokio::test(start_paused = true)]
    async fn forgets_a_session_vyprvpn_no_longer_has() {
        let vpn = Harness::in_dir(
            FakeApi {
                unknown_sessions: true,
                ..FakeApi::default()
            },
            FakeTunnel::default(),
            left_behind(&Saved {
                last_spoke_id: Some("de1".into()),
                session: Some(session("48213977")),
                pending_deletions: Vec::new(),
            }),
        );

        let_recovery_run_for(FIRST_RETRY / 2).await;

        assert_eq!(vpn.saved().pending_deletions, []);
    }

    #[tokio::test(start_paused = true)]
    async fn retries_a_delete_that_failed_on_disconnect() {
        let mut vpn = Harness::new(
            FakeApi {
                refused_closes: Mutex::new(1),
                ..FakeApi::default()
            },
            FakeTunnel::default(),
        );
        vpn.connect("de1");
        vpn.settled().await;
        vpn.vpn.disconnect();
        vpn.settled().await;

        let_recovery_run_for(FIRST_RETRY).await;

        assert_eq!(vpn.closed(), ["de1-1"]);
        assert_eq!(vpn.saved().pending_deletions, []);
    }

    #[tokio::test(start_paused = true)]
    async fn corrupt_record_is_reported_and_cleared() {
        let data_dir = tempfile::tempdir().unwrap();
        fs::write(data_dir.path().join("connection.json"), "{").unwrap();

        let vpn = Harness::in_dir(FakeApi::default(), FakeTunnel::default(), data_dir);

        let Status::Disconnected {
            last_spoke_id: None,
            error: Some(error),
        } = vpn.status.borrow().clone()
        else {
            panic!("{:?}", vpn.status.borrow());
        };
        assert!(error.contains("malformed"), "{error}");
        assert_eq!(vpn.saved(), Saved::default());
    }

    #[tokio::test(start_paused = true)]
    async fn recovers_the_tunnel_once_at_startup_before_any_connect() {
        let mut vpn = Harness::new(FakeApi::default(), FakeTunnel::default());
        vpn.tunnel.recovered.notified().await;

        vpn.connect("de1");
        vpn.settled().await;

        assert_eq!(*vpn.tunnel.recoveries.lock().unwrap(), 1);
        assert_eq!(*vpn.tunnel.ups.lock().unwrap(), 1);
    }
}
