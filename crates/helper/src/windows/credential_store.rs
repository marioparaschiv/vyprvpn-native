use std::fs;
use std::io;
use std::path::Path;

use windows_dpapi::Scope;

pub const CREDENTIALS_FILE: &str = "credentials.bin";

#[derive(Debug, thiserror::Error)]
#[error("DPAPI failed: {0}")]
pub struct SealError(String);

/// Encrypts with DPAPI for the account the helper runs as. Under LocalSystem only SYSTEM can
/// decrypt it, and the system profile's data directory keeps everyone but SYSTEM and
/// Administrators from reading the file at all.
pub fn seal(data: Vec<u8>) -> Result<Vec<u8>, SealError> {
    windows_dpapi::encrypt_data(&data, Scope::User, None)
        .map_err(|error| SealError(format!("{error:#}")))
}

pub fn unseal(sealed: Vec<u8>) -> Result<Vec<u8>, SealError> {
    windows_dpapi::decrypt_data(&sealed, Scope::User, None)
        .map_err(|error| SealError(format!("{error:#}")))
}

/// The data directory's ACL already keeps everyone else out.
pub fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    fs::write(path, contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential_store::CredentialStore;
    use crate::vpn::fakes::alice;

    #[test]
    fn encrypts_the_file() {
        let data_dir = tempfile::tempdir().unwrap();

        CredentialStore::new(data_dir.path())
            .save(&alice())
            .unwrap();
        let file = fs::read(data_dir.path().join(CREDENTIALS_FILE)).unwrap();

        for secret in ["alice", "correct horse"] {
            assert!(
                !file
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes()),
                "{secret} is stored in plain text"
            );
        }
    }
}
