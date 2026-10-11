mod wireguard;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, ExitStatus};

const BINARIES: [&str; 2] = ["VyprLite.exe", "vyprlite-helper.exe"];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("usage: cargo xtask dist")]
    Usage,
    #[error("{action} {}: {source}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to run {program}: {source}")]
    Spawn {
        program: &'static str,
        source: std::io::Error,
    },
    #[error("release build failed: {0}")]
    Build(ExitStatus),
    #[error("downloading {url}: {source}")]
    Download { url: String, source: reqwest::Error },
    #[error("SHA256 mismatch for {}: expected {expected}, got {actual}", path.display())]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("reading {entry} from {}: {source}", archive.display())]
    Zip {
        archive: PathBuf,
        entry: &'static str,
        source: zip::result::ZipError,
    },
    #[error("Get-AuthenticodeSignature failed ({status}): {stderr}")]
    SignatureQuery { status: ExitStatus, stderr: String },
    #[error("unexpected Get-AuthenticodeSignature output: {0:?}")]
    SignatureOutput(String),
    #[error("{} signature status is {status}, expected Valid (signer: {subject:?})", path.display())]
    SignatureInvalid {
        path: PathBuf,
        status: String,
        subject: String,
    },
    #[error("{} is signed by {subject:?}, expected CN={}", path.display(), wireguard::SIGNER)]
    UnexpectedSigner { path: PathBuf, subject: String },
}

fn main() -> ExitCode {
    let result = match env::args().nth(1).as_deref() {
        Some("dist") => dist(),
        _ => Err(Error::Usage),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn dist() -> Result<(), Error> {
    let root = workspace_root();
    let target_dir = env::var_os("CARGO_TARGET_DIR")
        .map(|dir| root.join(dir))
        .unwrap_or_else(|| root.join("target"));

    let dll = wireguard::fetch_verified_dll(&target_dir.join("wireguard-nt"))?;
    build_release(&root, &target_dir)?;

    let dist_dir = root.join("dist");
    if dist_dir.exists() {
        fs::remove_dir_all(&dist_dir).map_err(io_error("removing", &dist_dir))?;
    }
    fs::create_dir_all(&dist_dir).map_err(io_error("creating", &dist_dir))?;

    let release_dir = target_dir.join("release");
    for binary in BINARIES {
        copy(&release_dir.join(binary), &dist_dir.join(binary))?;
    }
    copy(&dll, &dist_dir.join(wireguard::DLL_NAME))?;

    println!("dist ready: {}", dist_dir.display());
    Ok(())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}

fn build_release(root: &Path, target_dir: &Path) -> Result<(), Error> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target_dir)
        .args(["build", "--release", "--package", "vyprlite-tray"])
        .args(["--package", "vyprlite-helper"])
        .status()
        .map_err(|source| Error::Spawn {
            program: "cargo",
            source,
        })?;

    if !status.success() {
        return Err(Error::Build(status));
    }

    Ok(())
}

fn copy(from: &Path, to: &Path) -> Result<(), Error> {
    fs::copy(from, to).map_err(io_error("copying", from))?;
    Ok(())
}

pub fn io_error(action: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.to_path_buf();
    move |source| Error::Io {
        action,
        path,
        source,
    }
}
