use std::process::{Command, ExitStatus};

pub const PLATFORM: &str = "Mac";

#[derive(Debug, thiserror::Error)]
pub enum OsVersionError {
    #[error("running sw_vers: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("sw_vers exited with {status}: {stderr}")]
    Failed { status: ExitStatus, stderr: String },
    #[error("sw_vers printed {0:?}, which is not a version number")]
    Unexpected(String),
}

/// The product version `sw_vers` prints, e.g. `14.5`.
pub fn os_version() -> Result<String, OsVersionError> {
    let output = Command::new("sw_vers").arg("-productVersion").output()?;

    if !output.status.success() {
        return Err(OsVersionError::Failed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let is_version = !version.is_empty()
        && version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.');
    if !is_version {
        return Err(OsVersionError::Unexpected(version));
    }

    Ok(version)
}
