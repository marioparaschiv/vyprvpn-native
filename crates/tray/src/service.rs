#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use std::env;
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use windows as platform;

use crate::app_dir::MissingEnv;
pub use platform::{installed, start};
#[cfg(windows)]
pub use windows::process_id;

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("finding the tray app's own directory failed: {0}")]
    ExeDir(io::Error),
    #[error("{} is missing, so the helper cannot be installed", .0.display())]
    MissingHelper(PathBuf),
    #[error(transparent)]
    MissingEnv(#[from] MissingEnv),
    #[error("asking for administrator rights failed: {0}")]
    Spawn(io::Error),
    #[error("installing the helper failed ({status}): {stderr}")]
    Failed { status: ExitStatus, stderr: String },
}

/// Installs the helper that ships beside the tray app, asking the user for administrator rights.
pub fn install() -> Result<(), InstallError> {
    let helper = env::current_exe()
        .map_err(InstallError::ExeDir)?
        .with_file_name(platform::HELPER_FILE);
    if !helper.is_file() {
        return Err(InstallError::MissingHelper(helper));
    }

    let output = platform::elevated_install(&helper)?
        .output()
        .map_err(InstallError::Spawn)?;

    if !output.status.success() {
        return Err(InstallError::Failed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    Ok(())
}
