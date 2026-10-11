use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use tunnel::{Teardown, TunnelError};

#[derive(Debug, thiserror::Error)]
pub enum TeardownStoreError {
    #[error("accessing {path} failed: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("the stored teardown is malformed: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("undoing the tunnel a previous helper left up failed: {0}")]
    Replay(#[from] TunnelError),
}

/// What undoes the tunnel that is up, kept on disk so a helper restarted after a crash can still
/// restore the routes and DNS.
#[derive(Debug, Clone)]
pub struct TeardownStore {
    path: PathBuf,
}

impl TeardownStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("teardown.json"),
        }
    }

    /// Call right after `tunnel::up`, before anything else can fail.
    pub fn save(&self, teardown: &Teardown) -> Result<(), TeardownStoreError> {
        let json = serde_json::to_vec(teardown)?;

        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&self.path)
            .and_then(|mut file| file.write_all(&json))
            .map_err(|source| self.io_error(source))
    }

    /// Call once the tunnel is down. Succeeds when there is nothing to forget.
    pub fn clear(&self) -> Result<(), TeardownStoreError> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(self.io_error(error)),
            _ => Ok(()),
        }
    }

    fn load(&self) -> Result<Option<Teardown>, TeardownStoreError> {
        let json = match fs::read(&self.path) {
            Ok(json) => json,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(self.io_error(source)),
        };

        Ok(Some(serde_json::from_slice(&json)?))
    }

    /// Undoes a tunnel left up by a helper that died, then forgets it. On failure the teardown
    /// stays stored, so the next start tries again.
    pub fn replay(&self) -> Result<(), TeardownStoreError> {
        let Some(teardown) = self.load()? else {
            return Ok(());
        };

        log::info!("undoing the tunnel a previous helper left up");
        teardown.run()?;

        self.clear()
    }

    fn io_error(&self, source: io::Error) -> TeardownStoreError {
        TeardownStoreError::Io {
            path: self.path.clone(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_nothing_when_no_tunnel_was_left_up() {
        let data_dir = tempfile::tempdir().unwrap();

        TeardownStore::new(data_dir.path()).replay().unwrap();
    }

    #[test]
    fn keeps_a_malformed_teardown_for_inspection() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = TeardownStore::new(data_dir.path());
        fs::write(&store.path, "{").unwrap();

        assert!(matches!(
            store.replay(),
            Err(TeardownStoreError::Malformed(_))
        ));
        assert!(store.path.exists());
    }

    #[test]
    fn clear_tolerates_nothing_stored() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = TeardownStore::new(data_dir.path());

        store.clear().unwrap();
        fs::write(&store.path, "{}").unwrap();
        store.clear().unwrap();

        assert!(!store.path.exists());
    }
}
