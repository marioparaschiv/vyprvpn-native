use std::backtrace::Backtrace;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::panic::{self, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Appends every panic to `path`, since a service has no console to print it on.
pub fn record_panics_in(path: PathBuf) {
    let default_hook = panic::take_hook();

    panic::set_hook(Box::new(move |info| {
        if let Err(error) = append(&path, info) {
            eprintln!("writing {} failed: {error}", path.display());
        }

        default_hook(info);
    }));
}

fn append(path: &Path, info: &PanicHookInfo) -> io::Result<()> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs();
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;

    writeln!(
        file,
        "[unix time {seconds}] {info}\n{}",
        Backtrace::force_capture()
    )
}

#[cfg(test)]
mod tests {
    use std::{env, fs, process, thread};

    use super::*;

    #[test]
    fn writes_panics_to_the_crash_log() {
        let path = env::temp_dir().join(format!("vyprlite-crash-log-test-{}.log", process::id()));
        record_panics_in(path.clone());

        let panicked = thread::spawn(|| panic!("tunnel exploded")).join();
        let log = fs::read_to_string(&path).unwrap();
        fs::remove_file(&path).unwrap();

        assert!(panicked.is_err());
        assert!(log.contains("tunnel exploded"), "{log}");
    }
}
