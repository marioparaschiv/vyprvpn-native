use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::{Error, io_error};

const ZIP_URL: &str = "https://download.wireguard.com/wireguard-nt/wireguard-nt-1.1.zip";
const ZIP_NAME: &str = "wireguard-nt-1.1.zip";
const ZIP_SHA256: &str = "dceb30a9bc4be48cce0f74160fc88a585a2c2627366e8f846fc6658f9038dace";
const DLL_ENTRY: &str = "wireguard-nt/bin/amd64/wireguard.dll";
pub const DLL_NAME: &str = "wireguard.dll";
pub const SIGNER: &str = "WireGuard LLC";

const SIGNED_FILE_VAR: &str = "VYPRVPN_SIGNED_FILE";
const SIGNATURE_SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
     $s = Get-AuthenticodeSignature -LiteralPath $env:VYPRVPN_SIGNED_FILE; \
     $s.Status.ToString(); $s.SignerCertificate.Subject";

/// Returns the path of the amd64 `wireguard.dll`, extracted from the pinned
/// WireGuardNT zip (downloaded once into `cache_dir`) after checking its hash
/// and Authenticode signature.
pub fn fetch_verified_dll(cache_dir: &Path) -> Result<PathBuf, Error> {
    let zip_path = cache_dir.join(ZIP_NAME);
    let archive = verified_zip(cache_dir, &zip_path)?;

    let dll = extract_entry(&archive, DLL_ENTRY, &zip_path)?;
    let dll_path = cache_dir.join(DLL_NAME);
    fs::write(&dll_path, dll).map_err(io_error("writing", &dll_path))?;

    let (status, subject) = query_signature(&dll_path)?;
    check_signature(&dll_path, &status, &subject)?;

    Ok(dll_path)
}

/// The cached zip when its hash matches, otherwise a fresh download that replaces it.
fn verified_zip(cache_dir: &Path, zip_path: &Path) -> Result<Vec<u8>, Error> {
    if zip_path.exists() {
        let cached = fs::read(zip_path).map_err(io_error("reading", zip_path))?;
        match verify_sha256(&cached, ZIP_SHA256, zip_path) {
            Ok(()) => return Ok(cached),
            Err(error) => eprintln!("{error}; downloading it again"),
        }
    }

    let archive = download(ZIP_URL)?;
    verify_sha256(&archive, ZIP_SHA256, zip_path)?;

    fs::create_dir_all(cache_dir).map_err(io_error("creating", cache_dir))?;
    fs::write(zip_path, &archive).map_err(io_error("writing", zip_path))?;

    Ok(archive)
}

pub(crate) fn download(url: &str) -> Result<Vec<u8>, Error> {
    let error = |source| Error::Download {
        url: url.to_owned(),
        source,
    };
    let body = reqwest::blocking::get(url)
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::bytes)
        .map_err(error)?;

    Ok(body.to_vec())
}

pub(crate) fn verify_sha256(bytes: &[u8], expected: &str, path: &Path) -> Result<(), Error> {
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();

    if actual != expected {
        return Err(Error::HashMismatch {
            path: path.to_path_buf(),
            expected: expected.to_owned(),
            actual,
        });
    }

    Ok(())
}

fn extract_entry(
    archive: &[u8],
    entry: &'static str,
    archive_path: &Path,
) -> Result<Vec<u8>, Error> {
    let zip_error = |source| Error::Zip {
        archive: archive_path.to_path_buf(),
        entry,
        source,
    };
    let mut zip = zip::ZipArchive::new(Cursor::new(archive)).map_err(zip_error)?;
    let mut file = zip.by_name(entry).map_err(zip_error)?;

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|source| zip_error(source.into()))?;

    Ok(bytes)
}

fn query_signature(path: &Path) -> Result<(String, String), Error> {
    // The path goes through the environment so it is never parsed as PowerShell. PowerShell 7
    // leaves its own module paths in PSModulePath, which Windows PowerShell then fails to load.
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            SIGNATURE_SCRIPT,
        ])
        .env(SIGNED_FILE_VAR, path)
        .env_remove("PSModulePath")
        .output()
        .map_err(|source| Error::Spawn {
            program: "powershell.exe",
            source,
        })?;

    if !output.status.success() {
        return Err(Error::SignatureQuery {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    parse_signature_output(&String::from_utf8_lossy(&output.stdout))
}

fn parse_signature_output(stdout: &str) -> Result<(String, String), Error> {
    let mut lines = stdout.lines().map(str::trim);
    let status = lines
        .next()
        .filter(|status| !status.is_empty())
        .ok_or_else(|| Error::SignatureOutput(stdout.to_owned()))?;
    let subject = lines.next().unwrap_or_default();

    Ok((status.to_owned(), subject.to_owned()))
}

fn check_signature(path: &Path, status: &str, subject: &str) -> Result<(), Error> {
    if status != "Valid" {
        return Err(Error::SignatureInvalid {
            path: path.to_path_buf(),
            status: status.to_owned(),
            subject: subject.to_owned(),
        });
    }

    let signer_cn = format!("CN={SIGNER}");
    if !subject.split(", ").any(|part| part == signer_cn) {
        return Err(Error::UnexpectedSigner {
            path: path.to_path_buf(),
            subject: subject.to_owned(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const WIREGUARD_SUBJECT: &str = "CN=WireGuard LLC, O=WireGuard LLC, S=Colorado, C=US, \
        OID.2.5.4.15=Private Organization, SERIALNUMBER=4227913";

    #[test]
    fn hash_check_accepts_matching_bytes() {
        verify_sha256(b"abc", ABC_SHA256, Path::new("abc.zip")).unwrap();
    }

    #[test]
    fn hash_check_rejects_other_bytes() {
        let err = verify_sha256(b"abd", ABC_SHA256, Path::new("abc.zip")).unwrap_err();

        let Error::HashMismatch {
            expected, actual, ..
        } = err
        else {
            panic!("expected HashMismatch, got {err:?}");
        };
        assert_eq!(expected, ABC_SHA256);
        assert_ne!(actual, ABC_SHA256);
        assert_eq!(actual.len(), 64);
    }

    #[test]
    fn parses_signed_and_unsigned_output() {
        let signed = parse_signature_output(&format!("Valid\r\n{WIREGUARD_SUBJECT}\r\n")).unwrap();
        assert_eq!(signed, ("Valid".to_owned(), WIREGUARD_SUBJECT.to_owned()));

        let unsigned = parse_signature_output("NotSigned\r\n").unwrap();
        assert_eq!(unsigned, ("NotSigned".to_owned(), String::new()));

        assert!(matches!(
            parse_signature_output(""),
            Err(Error::SignatureOutput(_))
        ));
    }

    #[test]
    fn accepts_valid_wireguard_signature() {
        check_signature(Path::new(DLL_NAME), "Valid", WIREGUARD_SUBJECT).unwrap();
    }

    #[test]
    fn rejects_invalid_status() {
        for status in ["NotSigned", "HashMismatch", "NotTrusted", "UnknownError"] {
            assert!(matches!(
                check_signature(Path::new(DLL_NAME), status, WIREGUARD_SUBJECT),
                Err(Error::SignatureInvalid { .. })
            ));
        }
    }

    #[test]
    fn rejects_other_signers() {
        for subject in [
            "",
            "CN=Evil Corp, O=Evil Corp",
            "CN=Evil Corp, O=WireGuard LLC",
            "CN=\"Evil, CN=WireGuard LLC\"",
        ] {
            assert!(matches!(
                check_signature(Path::new(DLL_NAME), "Valid", subject),
                Err(Error::UnexpectedSigner { .. })
            ));
        }
    }
}
