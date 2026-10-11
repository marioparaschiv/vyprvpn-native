use std::backtrace::Backtrace;
use std::fmt::Display;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::panic;
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

fn append(path: &Path, info: impl Display) -> io::Result<()> {
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
    use std::fs;

    use super::*;

    #[test]
    fn appends_each_panic_to_the_crash_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.log");

        append(&path, "tunnel exploded").unwrap();
        append(&path, "tunnel exploded again").unwrap();

        let log = fs::read_to_string(&path).unwrap();
        let first = log.find("] tunnel exploded\n").unwrap();
        let second = log.find("] tunnel exploded again\n").unwrap();
        assert!(first < second, "{log}");
    }
}
