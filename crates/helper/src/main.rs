#[cfg(not(any(windows, target_os = "macos")))]
compile_error!("vyprlite-helper supports only Windows and macOS");

mod connection;
mod crash_log;
mod disconnect;
#[cfg(target_os = "macos")]
mod macos;
mod server;
mod session_store;
mod state;
mod vpn;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use windows as platform;

fn main() -> std::process::ExitCode {
    platform::main()
}
