use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use crate::app_dir::{self, MissingEnv};

#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error(transparent)]
    MissingEnv(#[from] MissingEnv),
    #[error("locking {path} failed: {source}")]
    Lock { path: PathBuf, source: io::Error },
}

/// Locks `tray.lock` for as long as the returned file stays open, or returns `None` when another
/// tray app of this user already holds it.
pub fn claim() -> Result<Option<File>, InstanceError> {
    claim_in(&app_dir::cache_dir()?)
}

fn claim_in(dir: &Path) -> Result<Option<File>, InstanceError> {
    let path = dir.join("tray.lock");
    let failed = |source| InstanceError::Lock {
        path: path.clone(),
        source,
    };

    fs::create_dir_all(dir).map_err(failed)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(failed)?;

    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(source)) => Err(failed(source)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_claim_holds_until_it_is_released() {
        let dir = tempfile::tempdir().unwrap();

        let first = claim_in(dir.path()).unwrap();
        assert!(first.is_some());
        assert!(claim_in(dir.path()).unwrap().is_none());

        drop(first);
        assert!(claim_in(dir.path()).unwrap().is_some());
    }
}
