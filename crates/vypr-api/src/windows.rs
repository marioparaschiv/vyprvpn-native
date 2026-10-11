use std::convert::Infallible;

use windows_version::OsVersion;

pub const PLATFORM: &str = "Windows";

pub type OsVersionError = Infallible;

/// `major.minor.build`, as the official client reports it.
pub fn os_version() -> Result<String, OsVersionError> {
    let version = OsVersion::current();

    Ok(format!(
        "{}.{}.{}",
        version.major, version.minor, version.build
    ))
}
