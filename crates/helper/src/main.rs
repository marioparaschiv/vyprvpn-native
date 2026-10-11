#[cfg(not(any(windows, target_os = "macos")))]
compile_error!("vyprvpn-helper supports only Windows and macOS");

mod connection;
mod crash_log;
mod credential_store;
mod disconnect;
#[cfg(target_os = "macos")]
mod macos;
mod server;
mod session_store;
mod state;
#[cfg(test)]
mod test_support;
mod vpn;
#[cfg(windows)]
mod windows;

use std::env;
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use windows as platform;

fn main() -> ExitCode {
    match platform::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

/// Records panics in `crash.log` and logs to stderr, or to `helper.log` when `to_file`, since a
/// service has no console.
fn init_logging(data_dir: &Path, to_file: bool) -> io::Result<()> {
    crash_log::record_panics_in(data_dir.join("crash.log"));

    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if to_file {
        let log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(data_dir.join("helper.log"))?;
        logger.target(env_logger::Target::Pipe(Box::new(log_file)));
    }
    logger.init();

    Ok(())
}

/// Where the installer finds the files it installs.
fn exe_dir() -> io::Result<PathBuf> {
    env::current_exe()?
        .parent()
        .map(Path::to_owned)
        .ok_or_else(|| io::Error::other("the executable has no parent"))
}
