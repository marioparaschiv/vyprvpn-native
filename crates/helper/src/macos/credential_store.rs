use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use ipc::Credentials;

#[derive(Debug, thiserror::Error)]
pub enum CredentialStoreError {
    #[error("accessing {path} failed: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("the stored login is malformed: {0}")]
    Malformed(#[from] serde_json::Error),
}

/// The login as a root-only file. The daemon's data directory is root-owned with mode 0700, so
/// nobody else can reach the file either way.
#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("credentials.json"),
        }
    }

    pub fn load(&self) -> Result<Option<Credentials>, CredentialStoreError> {
        let json = match fs::read(&self.path) {
            Ok(json) => json,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(self.io_error(source)),
        };

        Ok(Some(serde_json::from_slice(&json)?))
    }

    pub fn save(&self, credentials: &Credentials) -> Result<(), CredentialStoreError> {
        let json = serde_json::to_vec(credentials)?;

        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&self.path)
            .and_then(|mut file| file.write_all(&json))
            .map_err(|source| self.io_error(source))
    }

    /// Succeeds when there is nothing to delete.
    pub fn delete(&self) -> Result<(), CredentialStoreError> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(self.io_error(error)),
            _ => Ok(()),
        }
    }

    fn io_error(&self, source: io::Error) -> CredentialStoreError {
        CredentialStoreError::Io {
            path: self.path.clone(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn alice() -> Credentials {
        Credentials {
            username: "alice".into(),
            password: "correct horse".into(),
        }
    }

    #[test]
    fn round_trips_a_login() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::new(data_dir.path());

        assert_eq!(store.load().unwrap(), None);
        store.save(&alice()).unwrap();

        assert_eq!(store.load().unwrap(), Some(alice()));
    }

    #[test]
    fn only_the_owner_can_read_the_file() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::new(data_dir.path());

        store.save(&alice()).unwrap();

        let mode = fs::metadata(&store.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn delete_removes_the_login_and_tolerates_none() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::new(data_dir.path());
        store.save(&alice()).unwrap();

        store.delete().unwrap();
        store.delete().unwrap();

        assert_eq!(store.load().unwrap(), None);
        assert!(!store.path.exists());
    }
}
