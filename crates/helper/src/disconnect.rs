use std::io;

use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::traits::tokio::Stream as _;
use interprocess::local_socket::{GenericFilePath, ToFsName};
use ipc::{Method, Outcome, Reply, Request, Response, ServerMessage, Status};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

#[derive(Debug, thiserror::Error)]
pub enum DisconnectError {
    #[error("asking the helper to disconnect failed: {0}")]
    Io(#[from] io::Error),
    #[error("the helper sent an unreadable reply to disconnect: {0}")]
    Reply(#[from] ipc::CodecError),
    #[error("the helper closed the connection before confirming the disconnect")]
    Unanswered,
    #[error("the helper could not disconnect: {0}")]
    Refused(String),
}

/// [`disconnect`] for the installers, which have no async runtime of their own.
pub fn disconnect_blocking(path: &str) -> Result<(), DisconnectError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(disconnect(path))
}

/// Asks the helper serving `path` to take the tunnel down and waits until it reports disconnected,
/// so the session and tunnel records it keeps are settled before the caller removes the helper. A
/// helper that is not running has nothing to disconnect.
async fn disconnect(path: &str) -> Result<(), DisconnectError> {
    let stream = match Stream::connect(path.to_fs_name::<GenericFilePath>()?).await {
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(());
        }
        result => result?,
    };

    disconnect_over(stream).await
}

async fn disconnect_over(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
) -> Result<(), DisconnectError> {
    let request = Request {
        id: 1,
        method: Method::Disconnect,
    };
    stream.write_all(&ipc::encode(&request)?).await?;

    let mut lines = BufReader::new(stream).lines();
    let mut answered = false;
    // Status pushes and the reply race on the helper's side, so the final status can arrive first.
    let mut pushed_disconnected = false;

    while let Some(line) = lines.next_line().await? {
        match ipc::decode(&line)? {
            ServerMessage::Response(Response { id, outcome }) if id == request.id => {
                match outcome {
                    Outcome::Result(Reply::Status(Status::Disconnected { .. })) => return Ok(()),
                    Outcome::Result(_) if pushed_disconnected => return Ok(()),
                    Outcome::Result(_) => answered = true,
                    Outcome::Error(reason) => return Err(DisconnectError::Refused(reason)),
                }
            }
            ServerMessage::Status(status) => {
                pushed_disconnected = matches!(status, Status::Disconnected { .. });
                if answered && pushed_disconnected {
                    return Ok(());
                }
            }
            _ => {}
        }
    }

    Err(DisconnectError::Unanswered)
}

#[cfg(test)]
mod tests {
    use tokio::io::{DuplexStream, Lines, ReadHalf, WriteHalf};
    use tokio::task::JoinHandle;

    use super::*;

    struct FakeHelper {
        requests: Lines<BufReader<ReadHalf<DuplexStream>>>,
        writer: WriteHalf<DuplexStream>,
    }

    impl FakeHelper {
        fn start() -> (Self, JoinHandle<Result<(), DisconnectError>>) {
            let (client, helper) = tokio::io::duplex(64 * 1024);
            let (reader, writer) = tokio::io::split(helper);
            let disconnecting = tokio::spawn(disconnect_over(client));

            (
                Self {
                    requests: BufReader::new(reader).lines(),
                    writer,
                },
                disconnecting,
            )
        }

        async fn receive(&mut self) -> Request {
            let line = self.requests.next_line().await.unwrap().expect("hung up");

            ipc::decode(&line).unwrap()
        }

        async fn send(&mut self, message: ServerMessage) {
            self.writer
                .write_all(&ipc::encode(&message).unwrap())
                .await
                .unwrap();
        }
    }

    fn disconnecting() -> Status {
        Status::Disconnecting {
            spoke_id: "de1".into(),
        }
    }

    fn disconnected() -> Status {
        Status::Disconnected {
            last_spoke_id: Some("de1".into()),
            error: None,
        }
    }

    fn reply(status: Status) -> ServerMessage {
        ServerMessage::Response(Response {
            id: 1,
            outcome: Outcome::Result(Reply::Status(status)),
        })
    }

    #[tokio::test]
    async fn waits_for_the_tunnel_to_be_down() {
        let (mut helper, disconnecting_task) = FakeHelper::start();

        assert_eq!(helper.receive().await.method, Method::Disconnect);
        helper.send(reply(disconnecting())).await;
        helper.send(ServerMessage::Status(disconnecting())).await;
        helper.send(ServerMessage::Status(disconnected())).await;

        disconnecting_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn accepts_the_final_status_pushed_before_the_reply() {
        let (mut helper, disconnecting_task) = FakeHelper::start();
        helper.receive().await;

        helper.send(ServerMessage::Status(disconnected())).await;
        helper.send(reply(disconnecting())).await;

        disconnecting_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn fails_when_the_helper_hangs_up_before_disconnecting() {
        let (mut helper, disconnecting_task) = FakeHelper::start();
        helper.receive().await;

        helper.send(reply(disconnecting())).await;
        drop(helper);

        assert!(matches!(
            disconnecting_task.await.unwrap(),
            Err(DisconnectError::Unanswered)
        ));
    }
}
