use std::future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::Listener as _;
use ipc::Status;
use tokio::task::JoinSet;

use crate::connection;
use crate::state::State;
use crate::vpn::Tunnel;

/// How long the helper waits without any tray app connected before it exits.
pub const IDLE_EXIT_AFTER: Duration = Duration::from_secs(5);

/// Serves every tray app that connects; returns once neither a tray app nor a tunnel has been up
/// for `idle_exit_after`.
pub async fn serve<T: Tunnel>(
    listener: Listener,
    state: Arc<State<T>>,
    idle_exit_after: Duration,
    admit: impl Fn(&Stream) -> io::Result<()>,
) -> io::Result<()> {
    let mut connections = JoinSet::new();
    let mut status = state.status();

    loop {
        let unused = connections.is_empty()
            && matches!(*status.borrow_and_update(), Status::Disconnected { .. });
        let idle = async {
            if unused {
                tokio::time::sleep(idle_exit_after).await;
            } else {
                future::pending::<()>().await;
            }
        };

        tokio::select! {
            accepted = listener.accept() => {
                let stream = accepted?;
                if let Err(error) = admit(&stream) {
                    log::warn!("refused a connection: {error}");
                    continue;
                }
                let state = state.clone();
                connections.spawn(async move {
                    if let Err(error) = connection::serve(stream, state).await {
                        log::warn!("dropped a tray app connection: {error}");
                    }
                });
            }
            Some(finished) = connections.join_next() => {
                if let Err(error) = finished {
                    log::error!("a tray app connection task failed: {error}");
                }
            }
            Ok(()) = status.changed() => {}
            () = idle => {
                log::info!("no tray app connected and no tunnel up for {idle_exit_after:?}; exiting");
                return Ok(());
            }
        }
    }
}
