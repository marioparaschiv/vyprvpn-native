use std::env;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::InstallError;
use crate::app_dir::MissingEnv;

pub const HELPER_FILE: &str = "vyprvpn-helper";

/// `vyprvpn-helper install` reads the installing user from sudo's variables, which the
/// administrator prompt does not set. Every argument is quoted so none is parsed as shell.
const INSTALL_SCRIPT: &str = r#"on run argv
	do shell script "SUDO_UID=" & quoted form of item 1 of argv & " SUDO_USER=" & quoted form of item 2 of argv & " " & quoted form of item 3 of argv & " install" with prompt "VyprVPN needs to install its helper." with administrator privileges
end run"#;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("the helper is not installed ({} is missing)", ipc::WAKE_PATH)]
    NotInstalled,
    #[error("waking the helper through {path} failed: {0}", path = ipc::WAKE_PATH)]
    Wake(io::Error),
    #[error("checking for {path} failed: {0}", path = ipc::WAKE_PATH)]
    Check(io::Error),
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

pub fn installed() -> Result<bool, ServiceError> {
    Path::new(ipc::WAKE_PATH)
        .try_exists()
        .map_err(ServiceError::Check)
}

pub fn elevated_install(helper: &Path) -> Result<Command, InstallError> {
    let user = env::var("USER").map_err(|_| MissingEnv("USER"))?;
    let mut command = Command::new("/usr/bin/osascript");
    command
        .args(["-e", INSTALL_SCRIPT])
        .arg(ipc::current_user_uid().to_string())
        .arg(user)
        .arg(helper);

    Ok(command)
}
