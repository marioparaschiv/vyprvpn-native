use std::sync::Arc;
use std::time::Duration;

use ipc::Status;

use super::{SessionApi, Tunnel, Vpn, describe};
use crate::session_store::{Saved, Session, SessionStore, SessionStoreError};

const FIRST_RETRY: Duration = Duration::from_secs(2);
const MAX_RETRY: Duration = Duration::from_secs(5 * 60);

/// Reads what the previous helper left, before any request is served. Its tunnel died with it,
/// so a recorded session moves to the pending deletions before a connect can overwrite it. A
/// corrupt record is reset, while one that cannot be read is left alone, since it may still hold
/// sessions to delete; either way the error is returned for the status.
pub(super) fn restore(store: &SessionStore) -> (Saved, Option<String>) {
    let mut saved = match store.load() {
        Ok(saved) => saved,
        Err(error @ SessionStoreError::Io { .. }) => {
            let message = format!("reading the saved connection failed: {error}");
            log::error!("{message}");

            return (Saved::default(), Some(message));
        }
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
        if let Err(error) = self.recover_tunnel().await {
            self.report_while_disconnected(error);
        }
        self.delete_pending_sessions().await;
    }

    /// Runs the platform's recovery once; a connect waits for it before bringing a tunnel up, and
    /// fails if what the previous helper's tunnel left could not be undone.
    pub(super) async fn recover_tunnel(&self) -> Result<(), String> {
        self.tunnel_recovered
            .get_or_init(|| async {
                let tunnel = self.tunnel.clone();
                let error = match tokio::task::spawn_blocking(move || tunnel.recover()).await {
                    Ok(Ok(())) => return Ok(()),
                    Ok(Err(error)) => describe(&error),
                    Err(error) => format!("undoing a leftover tunnel panicked: {error}"),
                };
                log::error!("{error}");

                Err(error)
            })
            .await
            .clone()
    }

    fn report_while_disconnected(&self, message: String) {
        let _target = self.target();

        self.status.send_if_modified(|status| match status {
            Status::Disconnected { error, .. } => {
                *error = Some(message);
                true
            }
            _ => false,
        });
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
                match self.close_session(session).await {
                    Ok(()) => deleted.push(session.clone()),
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

    use tokio::time::sleep;

    use super::*;
    use crate::vpn::fakes::{FakeApi, FakeTunnel, Harness, session};

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

        sleep(MAX_RETRY).await;

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

        sleep(FIRST_RETRY).await;

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

        sleep(FIRST_RETRY / 2).await;
        assert!(vpn.closed().is_empty());
        assert_eq!(vpn.saved().pending_deletions.len(), 2);

        sleep(FIRST_RETRY).await;
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

        sleep(FIRST_RETRY / 2).await;

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

        sleep(FIRST_RETRY).await;

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
    async fn unreadable_record_is_reported_and_kept() {
        let data_dir = tempfile::tempdir().unwrap();
        let record = data_dir.path().join("connection.json");
        fs::create_dir(&record).unwrap();

        let vpn = Harness::in_dir(FakeApi::default(), FakeTunnel::default(), data_dir);

        let Status::Disconnected {
            last_spoke_id: None,
            error: Some(error),
        } = vpn.status.borrow().clone()
        else {
            panic!("{:?}", vpn.status.borrow());
        };
        assert!(
            error.starts_with("reading the saved connection failed"),
            "{error}"
        );
        assert!(record.is_dir());
        assert_eq!(fs::read_dir(vpn.data_dir.path()).unwrap().count(), 1);
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

    #[tokio::test(start_paused = true)]
    async fn failed_tunnel_recovery_is_reported_and_fails_connects() {
        let mut vpn = Harness::new(
            FakeApi::default(),
            FakeTunnel {
                refuse_recovery: true,
                ..FakeTunnel::default()
            },
        );

        let reported = vpn
            .status
            .wait_for(|status| matches!(status, Status::Disconnected { error: Some(_), .. }))
            .await
            .unwrap()
            .clone();
        assert_eq!(
            reported,
            Status::Disconnected {
                last_spoke_id: None,
                error: Some("the fake adapter refused".into()),
            }
        );

        vpn.connect("de1");

        assert_eq!(
            vpn.settled().await,
            Status::Disconnected {
                last_spoke_id: Some("de1".into()),
                error: Some("the fake adapter refused".into()),
            }
        );
        assert!(vpn.opened().is_empty());
        assert_eq!(*vpn.tunnel.recoveries.lock().unwrap(), 1);
    }
}
