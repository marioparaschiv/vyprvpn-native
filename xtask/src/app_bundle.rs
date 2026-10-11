use std::fs;
use std::path::Path;
use std::process::Command;

use crate::{Error, copy, io_error};

const INFO_PLIST: &str = include_str!("../../crates/tray/assets/Info.plist");

/// Writes `VyprVPN.app` with both binaries side by side in `Contents/MacOS`, where
/// `vyprvpn-helper install` expects to find the tray app, and signs it ad hoc so Gatekeeper sees an
/// intact bundle.
pub fn write(assets_dir: &Path, release_dir: &Path, app: &Path) -> Result<(), Error> {
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    for dir in [&macos, &resources] {
        fs::create_dir_all(dir).map_err(io_error("creating", dir))?;
    }

    let plist = contents.join("Info.plist");
    fs::write(
        &plist,
        INFO_PLIST.replace("@VERSION@", env!("CARGO_PKG_VERSION")),
    )
    .map_err(io_error("writing", &plist))?;

    for binary in ["VyprVPN", "vyprvpn-helper"] {
        copy(&release_dir.join(binary), &macos.join(binary))?;
    }
    copy(
        &assets_dir.join("VyprVPN.icns"),
        &resources.join("VyprVPN.icns"),
    )?;

    // The nested helper first, since signing the bundle seals it as it stands.
    sign(&macos.join("vyprvpn-helper"))?;
    sign(app)
}

fn sign(path: &Path) -> Result<(), Error> {
    let status = Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(path)
        .status()
        .map_err(|source| Error::Spawn {
            program: "codesign",
            source,
        })?;

    if !status.success() {
        return Err(Error::Codesign {
            path: path.to_owned(),
            status,
        });
    }

    Ok(())
}
