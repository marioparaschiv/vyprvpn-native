use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ipc::Credentials;
use windows_dpapi::Scope;

#[derive(Debug, thiserror::Error)]
pub enum CredentialStoreError {
    #[error("accessing {path} failed: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("DPAPI failed: {0}")]
    Dpapi(String),
    #[error("the stored login is malformed: {0}")]
    Malformed(#[from] serde_json::Error),
}

/// The login, encrypted with DPAPI for the account the helper runs as. Under LocalSystem only
/// SYSTEM can decrypt it, and the system profile's data directory keeps everyone but SYSTEM and
/// Administrators from reading the file at all.
#[derive(Debug, Clone)]
pub struct CredentialStore {
    path: PathBuf,
}

impl CredentialStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("credentials.bin"),
        }
    }

    pub fn load(&self) -> Result<Option<Credentials>, CredentialStoreError> {
        let encrypted = match fs::read(&self.path) {
            Ok(encrypted) => encrypted,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(self.io_error(source)),
        };
        let json = windows_dpapi::decrypt_data(&encrypted, Scope::User, None)
            .map_err(|error| CredentialStoreError::Dpapi(format!("{error:#}")))?;

        Ok(Some(serde_json::from_slice(&json)?))
    }

    pub fn save(&self, credentials: &Credentials) -> Result<(), CredentialStoreError> {
        let encrypted =
            windows_dpapi::encrypt_data(&serde_json::to_vec(credentials)?, Scope::User, None)
                .map_err(|error| CredentialStoreError::Dpapi(format!("{error:#}")))?;

        fs::write(&self.path, encrypted).map_err(|source| self.io_error(source))
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
    fn encrypts_the_file() {
        let data_dir = tempfile::tempdir().unwrap();
        let store = CredentialStore::new(data_dir.path());

        store.save(&alice()).unwrap();
        let file = fs::read(&store.path).unwrap();

        for secret in ["alice", "correct horse"] {
            assert!(
                !file
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes()),
                "{secret} is stored in plain text"
            );
        }
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
