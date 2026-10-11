use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use windows_sys::Win32::Foundation::{GENERIC_ALL, GENERIC_WRITE};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_APPEND_DATA, FILE_DELETE_CHILD, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA,
    FILE_WRITE_EA, WRITE_DAC, WRITE_OWNER,
};

use super::security::Sid;

const SYSTEM: &str = "S-1-5-18";
const ADMINISTRATORS: &str = "S-1-5-32-544";
const TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

/// Rights that let an account change a file or folder, or grant itself that by rewriting the DACL.
const WRITE_RIGHTS: u32 = FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_DELETE_CHILD
    | FILE_WRITE_ATTRIBUTES
    | DELETE
    | WRITE_DAC
    | WRITE_OWNER
    | GENERIC_WRITE
    | GENERIC_ALL;

/// Rights on an ancestor that let an account rename it or a child out of the way and put its own
/// folder in place. Adding new entries, which Authenticated Users may do on `C:\`, cannot.
const REPLACE_RIGHTS: u32 = DELETE | FILE_DELETE_CHILD | WRITE_DAC | WRITE_OWNER | GENERIC_ALL;

const INHERIT_ONLY: u32 = 2;
const PATHS_VAR: &str = "VYPRVPN_ACL_PATHS";

/// Prints `owner <SID>` per path, then `ace <SID> <rights hex> <0 allow|1 deny> <propagation flags>`
/// per ACE. SIDs instead of account names, which are localized. Plain .NET instead of `Get-Acl`,
/// whose module fails to load when a PowerShell 7 prompt leaks its `PSModulePath`.
const ACL_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
$sid = [System.Security.Principal.SecurityIdentifier]
foreach ($path in $env:VYPRVPN_ACL_PATHS.Split("`n")) {
    if ([System.IO.Directory]::Exists($path)) { $item = [System.IO.DirectoryInfo]::new($path) } else { $item = [System.IO.FileInfo]::new($path) }
    $acl = $item.GetAccessControl()
    'owner ' + $acl.GetOwner($sid).Value
    foreach ($rule in $acl.GetAccessRules($true, $true, $sid)) {
        'ace {0} {1:X8} {2:D} {3:D}' -f $rule.IdentityReference.Value, [int]$rule.FileSystemRights, $rule.AccessControlType, $rule.PropagationFlags
    }
}"#;

#[derive(Debug, thiserror::Error)]
pub enum InstallDirError {
    #[error("running PowerShell to read the install folder's permissions failed: {0}")]
    Spawn(io::Error),
    #[error("reading the install folder's permissions failed ({status}): {stderr}")]
    Query { status: ExitStatus, stderr: String },
    #[error("unexpected output reading the install folder's permissions: {0:?}")]
    Output(String),
    #[error(
        "refusing to install: {} {exposure}. The helper runs as SYSTEM, so its folder must be writable only by administrators: from an elevated prompt, copy dist\\ to C:\\Program Files\\VyprVPN and run install from there",
        path.display()
    )]
    Unprotected { path: PathBuf, exposure: Exposure },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exposure {
    Owner(Sid),
    Writable { sid: Sid, rights: u32 },
}

impl fmt::Display for Exposure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Owner(sid) => write!(
                formatter,
                "is owned by {sid}, which can change its permissions"
            ),
            Self::Writable { sid, rights } => {
                write!(
                    formatter,
                    "can be changed by {sid} (access mask {rights:#010x})"
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Acl {
    owner: Sid,
    aces: Vec<Ace>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Ace {
    sid: Sid,
    rights: u32,
    allow: bool,
    inherit_only: bool,
}

/// Fails unless only SYSTEM, Administrators and TrustedInstaller can change `dir`, the `files` in
/// it, or which folder its path leads to.
pub fn check(system_root: &Path, dir: &Path, files: &[&str]) -> Result<(), InstallDirError> {
    let targets = targets(dir, files);
    let paths: Vec<_> = targets.iter().map(|(path, _)| path.as_path()).collect();
    let acls = query_acls(system_root, &paths)?;

    if acls.len() != targets.len() {
        return Err(InstallDirError::Output(format!(
            "{} ACLs for {} paths",
            acls.len(),
            targets.len()
        )));
    }

    for ((path, rights), acl) in targets.into_iter().zip(&acls) {
        if let Some(exposure) = exposure(acl, rights) {
            return Err(InstallDirError::Unprotected { path, exposure });
        }
    }

    Ok(())
}

fn targets(dir: &Path, files: &[&str]) -> Vec<(PathBuf, u32)> {
    let contents = files.iter().map(|file| dir.join(file));
    let ancestors = dir
        .ancestors()
        .skip(1)
        .map(|ancestor| (ancestor.to_owned(), REPLACE_RIGHTS));

    [dir.to_owned()]
        .into_iter()
        .chain(contents)
        .map(|path| (path, WRITE_RIGHTS))
        .chain(ancestors)
        .collect()
}

fn query_acls(system_root: &Path, paths: &[&Path]) -> Result<Vec<Acl>, InstallDirError> {
    // Windows paths cannot contain a newline, so it separates them safely.
    let joined = paths.iter().fold(OsString::new(), |mut joined, path| {
        if !joined.is_empty() {
            joined.push("\n");
        }
        joined.push(path);
        joined
    });

    // The full path, since PATH could lead an elevated prompt to another PowerShell. The paths go
    // through the environment so they are never parsed as PowerShell.
    let output = Command::new(system_root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-Command", ACL_SCRIPT])
        .env(PATHS_VAR, joined)
        .output()
        .map_err(InstallDirError::Spawn)?;

    if !output.status.success() {
        return Err(InstallDirError::Query {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    parse_acls(&String::from_utf8_lossy(&output.stdout))
}

fn parse_acls(stdout: &str) -> Result<Vec<Acl>, InstallDirError> {
    let mut acls: Vec<Acl> = Vec::new();

    for line in stdout.lines() {
        let invalid = || InstallDirError::Output(line.to_owned());

        match line.split_whitespace().collect::<Vec<_>>()[..] {
            [] => {}
            ["owner", owner] => acls.push(Acl {
                owner: Sid::parse(owner).map_err(|_| invalid())?,
                aces: Vec::new(),
            }),
            ["ace", sid, rights, kind, propagation] => {
                let ace = parse_ace(sid, rights, kind, propagation).ok_or_else(invalid)?;
                acls.last_mut().ok_or_else(invalid)?.aces.push(ace);
            }
            _ => return Err(invalid()),
        }
    }

    Ok(acls)
}

fn parse_ace(sid: &str, rights: &str, kind: &str, propagation: &str) -> Option<Ace> {
    let allow = match kind {
        "0" => true,
        "1" => false,
        _ => return None,
    };

    Some(Ace {
        sid: Sid::parse(sid).ok()?,
        rights: u32::from_str_radix(rights, 16).ok()?,
        allow,
        inherit_only: propagation.parse::<u32>().ok()? & INHERIT_ONLY != 0,
    })
}

/// The owner can always rewrite the DACL. Deny ACEs grant nothing, and inherit-only ACEs, such as
/// CREATOR OWNER's on Program Files, only apply to children.
fn exposure(acl: &Acl, rights: u32) -> Option<Exposure> {
    if !is_trusted(&acl.owner) {
        return Some(Exposure::Owner(acl.owner.clone()));
    }

    acl.aces
        .iter()
        .find(|ace| {
            ace.allow && !ace.inherit_only && ace.rights & rights != 0 && !is_trusted(&ace.sid)
        })
        .map(|ace| Exposure::Writable {
            sid: ace.sid.clone(),
            rights: ace.rights,
        })
}

fn is_trusted(sid: &Sid) -> bool {
    [SYSTEM, ADMINISTRATORS, TRUSTED_INSTALLER].contains(&sid.to_string().as_str())
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    const ALICE: &str = "S-1-5-21-1309070409-676813193-2614181977-1002";

    /// `C:\Program Files`; icacls shows TrustedInstaller (F), SYSTEM and Administrators (M) plus
    /// (OI)(CI)(IO)(F), Users and both app package groups (RX), and CREATOR OWNER (OI)(CI)(IO)(F).
    const PROGRAM_FILES: &str = "\
owner S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464
ace S-1-3-0 10000000 0 2
ace S-1-5-18 10000000 0 2
ace S-1-5-18 001301BF 0 0
ace S-1-5-32-544 10000000 0 2
ace S-1-5-32-544 001301BF 0 0
ace S-1-5-32-545 A0000000 0 2
ace S-1-5-32-545 001200A9 0 0
ace S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464 10000000 0 2
ace S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464 001F01FF 0 0
ace S-1-15-2-1 001200A9 0 0
ace S-1-15-2-1 A0000000 0 2
ace S-1-15-2-2 001200A9 0 0
ace S-1-15-2-2 A0000000 0 2
";

    /// `C:\`; icacls shows `Authenticated Users:(OI)(CI)(IO)(M)` and `Authenticated Users:(AD)`.
    const DRIVE_ROOT: &str = "\
owner S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464
ace S-1-5-11 00000004 0 0
ace S-1-5-11 E0010000 0 2
ace S-1-5-18 001F01FF 0 0
ace S-1-5-32-544 001F01FF 0 0
ace S-1-5-32-545 001200A9 0 0
ace S-1-15-3-65536-1888954469-739942743-1668119174-2468466756-4239452838-1296943325-355587736-700089176 001000A1 0 0
";

    /// A clone in a profile; icacls shows `DESKTOP\alice:(I)(OI)(CI)(F)`.
    const USER_DIR: &str = "\
owner S-1-5-21-1309070409-676813193-2614181977-1002
ace S-1-5-18 001F01FF 0 0
ace S-1-5-32-544 001F01FF 0 0
ace S-1-5-21-1309070409-676813193-2614181977-1002 001F01FF 0 0
";

    fn sid(text: &str) -> Sid {
        Sid::parse(text).unwrap()
    }

    fn acl(stdout: &str) -> Acl {
        let mut acls = parse_acls(stdout).unwrap();
        assert_eq!(acls.len(), 1);
        acls.remove(0)
    }

    /// An admin-owned folder whose DACL is Program Files' plus `ace`.
    fn program_files_with(ace: &str) -> Acl {
        acl(&format!(
            "{}{ace}\n",
            PROGRAM_FILES.replace(
                "owner S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464",
                "owner S-1-5-32-544"
            )
        ))
    }

    #[test]
    fn parses_owner_and_aces_per_path() {
        let acls = parse_acls(&format!("{DRIVE_ROOT}{USER_DIR}")).unwrap();

        assert_eq!(acls.len(), 2);
        assert_eq!(acls[0].owner, sid(TRUSTED_INSTALLER));
        assert_eq!(
            acls[0].aces[1],
            Ace {
                sid: sid("S-1-5-11"),
                rights: 0xE001_0000,
                allow: true,
                inherit_only: true,
            }
        );
        assert_eq!(acls[1].owner, sid(ALICE));
        assert_eq!(acls[1].aces.len(), 3);
    }

    #[test]
    fn parses_crlf_and_deny_aces() {
        let acls = parse_acls("owner S-1-5-18\r\nace S-1-1-0 00000002 1 3\r\n").unwrap();

        assert_eq!(
            acls[0].aces,
            [Ace {
                sid: sid("S-1-1-0"),
                rights: 0x2,
                allow: false,
                inherit_only: true,
            }]
        );
    }

    #[test]
    fn rejects_malformed_output() {
        for stdout in [
            "ace S-1-5-18 001F01FF 0 0\n",
            "owner alice\n",
            "owner S-1-5-18\nace S-1-5-18 zz 0 0\n",
            "owner S-1-5-18\nace S-1-5-18 001F01FF Allow None\n",
            "WARNING: something\n",
        ] {
            assert!(
                matches!(parse_acls(stdout), Err(InstallDirError::Output(_))),
                "{stdout:?}"
            );
        }
    }

    #[test]
    fn accepts_program_files_and_its_inherit_only_creator_owner() {
        assert_eq!(exposure(&acl(PROGRAM_FILES), WRITE_RIGHTS), None);
    }

    #[test]
    fn rejects_a_folder_its_user_owns() {
        assert_eq!(
            exposure(&acl(USER_DIR), WRITE_RIGHTS),
            Some(Exposure::Owner(sid(ALICE)))
        );
    }

    #[test]
    fn rejects_a_user_with_full_control_even_when_admins_own_the_folder() {
        let dir = acl(&USER_DIR.replace(&format!("owner {ALICE}"), "owner S-1-5-32-544"));

        assert_eq!(
            exposure(&dir, WRITE_RIGHTS),
            Some(Exposure::Writable {
                sid: sid(ALICE),
                rights: 0x001F_01FF,
            })
        );
    }

    #[test]
    fn rejects_any_write_type_right_for_other_accounts() {
        let cases = [
            ("S-1-5-32-545", "001301BF", "BUILTIN\\Users:(M)"),
            ("S-1-5-11", "001301BF", "Authenticated Users:(M)"),
            ("S-1-1-0", "00100116", "Everyone:(W)"),
            ("S-1-5-32-545", "001201BF", "(OI)(CI)(RX,W)"),
            ("S-1-5-32-545", "00000004", "(AD)"),
            ("S-1-5-32-545", "00000002", "(WD)"),
            ("S-1-5-32-545", "00000040", "(DC)"),
            ("S-1-5-32-545", "00010000", "(DE)"),
            ("S-1-5-32-545", "00040000", "(WDAC)"),
            ("S-1-5-32-545", "00080000", "(WO)"),
            ("S-1-5-32-545", "40000000", "(GW)"),
            ("S-1-5-32-545", "10000000", "(GA)"),
        ];

        for (who, rights, icacls) in cases {
            assert!(
                matches!(
                    exposure(&program_files_with(&format!("ace {who} {rights} 0 0")), WRITE_RIGHTS),
                    Some(Exposure::Writable { sid, .. }) if sid.to_string() == who
                ),
                "{icacls}"
            );
        }
    }

    #[test]
    fn ignores_deny_and_inherit_only_aces() {
        for ace in ["ace S-1-1-0 001F01FF 1 0", "ace S-1-5-32-545 001F01FF 0 2"] {
            assert_eq!(
                exposure(&program_files_with(ace), WRITE_RIGHTS),
                None,
                "{ace}"
            );
        }
    }

    #[test]
    fn rejects_creator_owner_when_it_applies_to_the_folder_itself() {
        assert!(matches!(
            exposure(
                &program_files_with("ace S-1-3-0 10000000 0 0"),
                WRITE_RIGHTS
            ),
            Some(Exposure::Writable { .. })
        ));
    }

    #[test]
    fn an_ancestor_may_let_users_add_entries_but_not_rename_them() {
        assert_eq!(exposure(&acl(DRIVE_ROOT), REPLACE_RIGHTS), None);
        assert!(exposure(&acl(DRIVE_ROOT), WRITE_RIGHTS).is_some());

        for (rights, icacls) in [("00000040", "(DC)"), ("00010000", "(DE)")] {
            assert!(
                exposure(
                    &program_files_with(&format!("ace S-1-5-11 {rights} 0 0")),
                    REPLACE_RIGHTS
                )
                .is_some(),
                "{icacls}"
            );
        }
    }

    #[test]
    fn checks_the_folder_and_files_for_writes_and_every_ancestor_for_renames() {
        assert_eq!(
            targets(
                Path::new(r"C:\Program Files\VyprVPN"),
                &["vyprvpn-helper.exe", "wireguard.dll"]
            ),
            [
                (PathBuf::from(r"C:\Program Files\VyprVPN"), WRITE_RIGHTS),
                (
                    PathBuf::from(r"C:\Program Files\VyprVPN\vyprvpn-helper.exe"),
                    WRITE_RIGHTS
                ),
                (
                    PathBuf::from(r"C:\Program Files\VyprVPN\wireguard.dll"),
                    WRITE_RIGHTS
                ),
                (PathBuf::from(r"C:\Program Files"), REPLACE_RIGHTS),
                (PathBuf::from(r"C:\"), REPLACE_RIGHTS),
            ]
        );
    }

    fn system_root() -> PathBuf {
        env::var_os("SystemRoot").unwrap().into()
    }

    #[test]
    fn accepts_a_stock_system_folder() {
        check(
            &system_root(),
            &system_root().join("System32"),
            &["cmd.exe"],
        )
        .unwrap();
    }

    #[test]
    fn refuses_a_folder_the_current_user_created() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("vyprvpn-helper.exe"), b"").unwrap();

        assert!(matches!(
            check(&system_root(), dir.path(), &["vyprvpn-helper.exe"]),
            Err(InstallDirError::Unprotected { path, .. }) if path == dir.path()
        ));
    }

    #[test]
    fn fails_on_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();

        assert!(matches!(
            check(&system_root(), dir.path(), &["wireguard.dll"]),
            Err(InstallDirError::Query { .. })
        ));
    }
}
