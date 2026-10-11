use std::path::PathBuf;

/// The socket the launchd daemon serves.
pub const DAEMON_SOCKET_PATH: &str = "/var/run/vyprlite.sock";

/// launchd starts the daemon whenever this file changes, so the tray app touches it when nothing
/// answers on [`DAEMON_SOCKET_PATH`]. The installer creates it, writable only by the owner.
pub const WAKE_PATH: &str = "/var/db/vyprlite.wake";

/// The socket a `--console` helper serves on. macOS gives every user a private temporary
/// directory, so it never collides with the daemon or another user's helper.
pub fn console_socket_path() -> PathBuf {
    std::env::temp_dir().join("vyprlite-console.sock")
}

/// The uid this process runs as.
pub fn current_user_uid() -> u32 {
    rustix::process::getuid().as_raw()
}
