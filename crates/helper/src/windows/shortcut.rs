use std::io;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};

use super::install::{self, InstallError, TRAY_EXE};

const TRAY_VAR: &str = "VYPRVPN_TRAY";

/// The desktop is looked up rather than built from `%USERPROFILE%`, since OneDrive can move it.
/// The tray path goes through the environment so it is never parsed as PowerShell.
const SHORTCUT_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
$desktop = [Environment]::GetFolderPath('Desktop')
$shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path $desktop 'VyprVPN.lnk'))
$shortcut.TargetPath = $env:VYPRVPN_TRAY
$shortcut.WorkingDirectory = Split-Path $env:VYPRVPN_TRAY
$shortcut.IconLocation = "$env:VYPRVPN_TRAY,0"
$shortcut.Save()
$shortcut.FullName"#;

#[derive(Debug, thiserror::Error)]
pub enum ShortcutError {
    #[error("finding the helper's own directory failed: {0}")]
    ExeDir(io::Error),
    #[error("{} is missing; run this from the folder VyprVPN is installed in", .0.display())]
    MissingTray(PathBuf),
    #[error(transparent)]
    SystemRoot(#[from] InstallError),
    #[error("running PowerShell failed: {0}")]
    Spawn(io::Error),
    #[error("creating the shortcut failed ({status}): {stderr}")]
    Create { status: ExitStatus, stderr: String },
}

/// Puts a VyprVPN shortcut on the current user's desktop, pointing at the tray app beside the
/// helper, with the icon the tray app carries.
pub fn create() -> Result<(), ShortcutError> {
    let tray = crate::exe_dir()
        .map_err(ShortcutError::ExeDir)?
        .join(TRAY_EXE);
    if !tray.is_file() {
        return Err(ShortcutError::MissingTray(tray));
    }

    let output = Command::new(
        install::system_root()?.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"),
    )
    .args(["-NoProfile", "-NonInteractive", "-Command", SHORTCUT_SCRIPT])
    .env(TRAY_VAR, &tray)
    .output()
    .map_err(ShortcutError::Spawn)?;

    if !output.status.success() {
        return Err(ShortcutError::Create {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    println!(
        "Created {}.",
        String::from_utf8_lossy(&output.stdout).trim()
    );

    Ok(())
}
