use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ipc::Credentials;

use crate::platform::{CREDENTIALS_FILE, SealError, seal, unseal, write_private};

#[derive(Debug, thiserror::Error)]
pub enum CredentialStoreError {
    #[error("accessing {path} failed: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("protecting the stored login failed: {0}")]
    Seal(#[source] SealError),
    #[error("the stored login is malformed: {0}")]
    Malformed(#[from] serde_json::Error),
}

/// The login in the helper's data directory, sealed and written as the platform module decides.
#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join(CREDENTIALS_FILE),
        }
    }

    pub fn load(&self) -> Result<Option<Credentials>, CredentialStoreError> {
        let sealed = match fs::read(&self.path) {
            Ok(sealed) => sealed,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(self.io_error(source)),
        };
        let json = unseal(sealed).map_err(CredentialStoreError::Seal)?;

        Ok(Some(serde_json::from_slice(&json)?))
    }

    pub fn save(&self, credentials: &Credentials) -> Result<(), CredentialStoreError> {
        let sealed = seal(serde_json::to_vec(credentials)?).map_err(CredentialStoreError::Seal)?;

        write_private(&self.path, &sealed).map_err(|source| self.io_error(source))
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
    use super::*;
    use crate::vpn::fakes::alice;

    #[test]
    fn round_trips_a_login() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::new(data_dir.path());

        assert_eq!(store.load().unwrap(), None);
        store.save(&alice()).unwrap();

        assert_eq!(store.load().unwrap(), Some(alice()));
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
