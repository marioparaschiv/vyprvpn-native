use std::env;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
#[error("environment variable {0} is not set")]
pub struct MissingEnv(pub &'static str);

/// `%LOCALAPPDATA%\VyprVPN` on Windows, `~/Library/Caches/VyprVPN` on macOS.
pub fn cache_dir() -> Result<PathBuf, MissingEnv> {
    Ok(caches_root()?.join("VyprVPN"))
}

#[cfg(windows)]
fn caches_root() -> Result<PathBuf, MissingEnv> {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or(MissingEnv("LOCALAPPDATA"))
}

#[cfg(target_os = "macos")]
fn caches_root() -> Result<PathBuf, MissingEnv> {
    let home = env::var_os("HOME").ok_or(MissingEnv("HOME"))?;

    Ok(PathBuf::from(home).join("Library").join("Caches"))
}
