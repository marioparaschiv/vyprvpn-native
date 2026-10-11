use std::io;

use interprocess::local_socket::Name;
use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::traits::tokio::Stream as _;
use ipc::{Method, Outcome, Request, Response, ServerMessage};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

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

/// Asks the helper serving `name` to take the tunnel down. A helper that is not running has
/// nothing to disconnect.
pub async fn disconnect(name: Name<'_>) -> Result<(), DisconnectError> {
    let mut stream = match Stream::connect(name).await {
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

    let request = Request {
        id: 1,
        method: Method::Disconnect,
    };
    stream.write_all(&ipc::encode(&request)?).await?;

    let mut lines = BufReader::new(&mut stream).lines();
    while let Some(line) = lines.next_line().await? {
        if let ServerMessage::Response(Response { id, outcome }) = ipc::decode(&line)?
            && id == request.id
        {
            return match outcome {
                Outcome::Result(_) => Ok(()),
                Outcome::Error(reason) => Err(DisconnectError::Refused(reason)),
            };
        }
    }

    Err(DisconnectError::Unanswered)
}
