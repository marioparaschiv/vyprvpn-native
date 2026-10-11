use std::convert::Infallible;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub const CREDENTIALS_FILE: &str = "credentials.json";

pub type SealError = Infallible;

/// Stored as is: the file is root-only, and the daemon's data directory is root-owned with mode
/// 0700, so nobody else can reach it either way.
pub fn seal(data: Vec<u8>) -> Result<Vec<u8>, SealError> {
    Ok(data)
}

pub fn unseal(sealed: Vec<u8>) -> Result<Vec<u8>, SealError> {
    Ok(sealed)
}

/// Readable and writable only by the helper's account.
pub fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(contents))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::credential_store::CredentialStore;
    use crate::vpn::fakes::alice;

    #[test]
    fn only_the_owner_can_read_the_file() {
        let data_dir = tempfile::tempdir().unwrap();

        CredentialStore::new(data_dir.path())
            .save(&alice())
            .unwrap();

        let mode = fs::metadata(data_dir.path().join(CREDENTIALS_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
