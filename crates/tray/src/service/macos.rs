use std::fs::File;
use std::io::{self, Write as _};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("the helper is not installed ({} is missing)", ipc::WAKE_PATH)]
    NotInstalled,
    #[error("waking the helper through {path} failed: {0}", path = ipc::WAKE_PATH)]
    Wake(io::Error),
}

/// launchd starts the helper daemon whenever its wake file is written; the installer creates the
/// file, writable by the installing user. The helper ignores what it says.
pub fn start() -> Result<(), ServiceError> {
    let mut wake = File::options()
        .write(true)
        .truncate(true)
        .open(ipc::WAKE_PATH)
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => ServiceError::NotInstalled,
            _ => ServiceError::Wake(error),
        })?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    writeln!(wake, "{now}").map_err(ServiceError::Wake)
}
