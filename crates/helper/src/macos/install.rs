use std::env;
use std::ffi::OsString;
use std::fs::{self, OpenOptions, Permissions};
use std::io::{self, Write};
use std::num::ParseIntError;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, chown};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use interprocess::local_socket::{GenericFilePath, ToFsName};

use super::DAEMON_DATA_DIR;
use crate::disconnect::{DisconnectError, disconnect};

const HELPER_PATH: &str = "/Library/PrivilegedHelperTools/com.vyprlite.helper";
const TRAY_DIR: &str = "/usr/local/libexec/vyprlite";
const TRAY_PATH: &str = "/usr/local/libexec/vyprlite/VyprLite";
const DAEMON_LABEL: &str = "com.vyprlite.helper";
const DAEMON_PLIST_PATH: &str = "/Library/LaunchDaemons/com.vyprlite.helper.plist";
const AGENT_PLIST: &str = "Library/LaunchAgents/com.vyprlite.tray.plist";
const DAEMON_PLIST: &str = include_str!("../../macos/com.vyprlite.helper.plist");
const AGENT_PLIST_TEXT: &str = include_str!("../../macos/com.vyprlite.tray.plist");
const LAUNCHCTL: &str = "/bin/launchctl";
const DSCL: &str = "/usr/bin/dscl";
/// What `launchctl print` exits with for a service that is not loaded.
const LAUNCHCTL_NOT_FOUND: i32 = 113;

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("permission denied: run this with sudo")]
    NotRoot,
    #[error("run this with sudo from the account that will use VyprLite ({0} is not set)")]
    NotSudo(&'static str),
    #[error("run sudo from the account that will use VyprLite, not from a root shell")]
    RootOwner,
    #[error("SUDO_UID is not a uid: {0}")]
    InvalidUid(#[from] ParseIntError),
    #[error("finding the helper's own directory failed: {0}")]
    ExeDir(io::Error),
    #[error("{action} {path} failed: {source}")]
    File {
        action: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    #[error("running {command} failed: {source}")]
    Spawn { command: String, source: io::Error },
    #[error("{command} failed ({status}): {output}")]
    CommandFailed {
        command: String,
        status: String,
        output: String,
    },
    #[error("dscl did not report a home directory for {user}: {output}")]
    HomeDir { user: String, output: String },
    #[error(transparent)]
    Disconnect(#[from] DisconnectError),
}

/// The user who ran `sudo`; the helper admits them and the tray app starts at their login.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Owner {
    uid: u32,
    home: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Replaces `to` through a rename, so a running copy keeps its file.
    Copy {
        from: PathBuf,
        to: PathBuf,
    },
    /// Creates the parent directory for `owner` if it is missing.
    Write {
        path: PathBuf,
        contents: String,
        owner: u32,
    },
    /// The tray app writes it to have launchd start the helper; only the owner may.
    CreateWakeFile {
        owner: u32,
    },
    Disconnect,
    /// Stops the helper and unloads it, so neither the tray app nor launchd can start it again.
    Bootout,
    Bootstrap,
    Remove(PathBuf),
    RemoveDir(PathBuf),
}

fn install_plan(exe_dir: &Path, owner: &Owner, daemon_loaded: bool) -> Vec<Step> {
    let mut plan = vec![
        Step::Copy {
            from: exe_dir.join("vyprlite-helper"),
            to: HELPER_PATH.into(),
        },
        Step::Copy {
            from: exe_dir.join("VyprLite"),
            to: TRAY_PATH.into(),
        },
        Step::Write {
            path: DAEMON_PLIST_PATH.into(),
            contents: daemon_plist(owner.uid),
            owner: 0,
        },
        Step::CreateWakeFile { owner: owner.uid },
    ];

    if daemon_loaded {
        plan.push(Step::Bootout);
    }
    plan.extend([
        Step::Bootstrap,
        Step::Write {
            path: owner.home.join(AGENT_PLIST),
            contents: AGENT_PLIST_TEXT.to_owned(),
            owner: owner.uid,
        },
    ]);

    plan
}

fn uninstall_plan(home: &Path, daemon_loaded: bool, exists: impl Fn(&Path) -> bool) -> Vec<Step> {
    let mut plan = Vec::new();

    if exists(Path::new(ipc::DAEMON_SOCKET_PATH)) {
        plan.push(Step::Disconnect);
    }
    if daemon_loaded {
        plan.push(Step::Bootout);
    }

    let files = [
        home.join(AGENT_PLIST),
        DAEMON_PLIST_PATH.into(),
        ipc::WAKE_PATH.into(),
        ipc::DAEMON_SOCKET_PATH.into(),
        HELPER_PATH.into(),
    ];
    plan.extend(
        files
            .into_iter()
            .filter(|path| exists(path))
            .map(Step::Remove),
    );
    plan.extend(
        [PathBuf::from(TRAY_DIR), PathBuf::from(DAEMON_DATA_DIR)]
            .into_iter()
            .filter(|path| exists(path))
            .map(Step::RemoveDir),
    );

    plan
}

fn daemon_plist(owner: u32) -> String {
    DAEMON_PLIST.replace("@OWNER_UID@", &owner.to_string())
}

/// Parses `dscl . -read /Users/<user> NFSHomeDirectory`.
fn parse_home(output: &str) -> Option<PathBuf> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("NFSHomeDirectory:"))
        .map(str::trim)
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

pub fn install() -> Result<(), InstallError> {
    let owner = sudo_owner()?;
    let exe_dir = env::current_exe()
        .map_err(InstallError::ExeDir)?
        .parent()
        .ok_or_else(|| InstallError::ExeDir(io::Error::other("the executable has no parent")))?
        .to_owned();

    execute(&install_plan(&exe_dir, &owner, daemon_loaded()?))?;
    println!("VyprLite is installed and starts at your next login.");

    Ok(())
}

pub fn uninstall() -> Result<(), InstallError> {
    let owner = sudo_owner()?;
    let plan = uninstall_plan(&owner.home, daemon_loaded()?, |path| {
        path.symlink_metadata().is_ok()
    });

    execute(&plan)?;
    println!("VyprLite is uninstalled.");

    Ok(())
}

fn sudo_owner() -> Result<Owner, InstallError> {
    if ipc::current_user_uid() != 0 {
        return Err(InstallError::NotRoot);
    }

    let uid: u32 = env::var("SUDO_UID")
        .map_err(|_| InstallError::NotSudo("SUDO_UID"))?
        .parse()?;
    if uid == 0 {
        return Err(InstallError::RootOwner);
    }

    let user = env::var("SUDO_USER").map_err(|_| InstallError::NotSudo("SUDO_USER"))?;
    let output = run(
        DSCL,
        &[".", "-read", &format!("/Users/{user}"), "NFSHomeDirectory"],
    )?;
    let output = String::from_utf8_lossy(&output.stdout);
    let home = parse_home(&output).ok_or_else(|| InstallError::HomeDir {
        user,
        output: output.trim().to_owned(),
    })?;

    Ok(Owner { uid, home })
}

fn daemon_loaded() -> Result<bool, InstallError> {
    let target = format!("system/{DAEMON_LABEL}");
    let output = spawn(LAUNCHCTL, &["print", &target])?;

    match output.status.code() {
        Some(0) => Ok(true),
        Some(LAUNCHCTL_NOT_FOUND) => Ok(false),
        _ => Err(command_failed(LAUNCHCTL, &["print", &target], &output)),
    }
}

fn execute(plan: &[Step]) -> Result<(), InstallError> {
    for step in plan {
        match step {
            Step::Copy { from, to } => copy(from, to)?,
            Step::Write {
                path,
                contents,
                owner,
            } => write(path, contents, *owner)?,
            Step::CreateWakeFile { owner } => create_wake_file(*owner)?,
            Step::Disconnect => tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(DisconnectError::Io)?
                .block_on(disconnect(
                    ipc::DAEMON_SOCKET_PATH
                        .to_fs_name::<GenericFilePath>()
                        .map_err(DisconnectError::Io)?,
                ))?,
            Step::Bootout => {
                run(LAUNCHCTL, &["bootout", &format!("system/{DAEMON_LABEL}")])?;
            }
            Step::Bootstrap => {
                run(LAUNCHCTL, &["bootstrap", "system", DAEMON_PLIST_PATH])?;
            }
            Step::Remove(path) => match fs::remove_file(path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => {
                    return Err(file_error("deleting", path)(error));
                }
                _ => {}
            },
            Step::RemoveDir(path) => match fs::remove_dir_all(path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => {
                    return Err(file_error("deleting", path)(error));
                }
                _ => {}
            },
        }
    }

    Ok(())
}

fn file_error(action: &'static str, path: &Path) -> impl Fn(io::Error) -> InstallError {
    move |source| InstallError::File {
        action,
        path: path.to_owned(),
        source,
    }
}

/// Root-owned and not writable by anyone else, since launchd runs the helper as root.
fn copy(from: &Path, to: &Path) -> Result<(), InstallError> {
    let mut staged = OsString::from(to);
    staged.push(".new");
    let staged = PathBuf::from(staged);

    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(file_error("creating", parent))?;
    }
    fs::copy(from, &staged).map_err(file_error("copying to", &staged))?;
    chown(&staged, Some(0), Some(0)).map_err(file_error("changing the owner of", &staged))?;
    fs::set_permissions(&staged, Permissions::from_mode(0o755))
        .map_err(file_error("changing the mode of", &staged))?;

    fs::rename(&staged, to).map_err(file_error("replacing", to))
}

fn write(path: &Path, contents: &str, owner: u32) -> Result<(), InstallError> {
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(file_error("creating", parent))?;
        chown(parent, Some(owner), None).map_err(file_error("changing the owner of", parent))?;
    }

    fs::write(path, contents).map_err(file_error("writing", path))?;
    chown(path, Some(owner), None).map_err(file_error("changing the owner of", path))?;

    fs::set_permissions(path, Permissions::from_mode(0o644))
        .map_err(file_error("changing the mode of", path))
}

fn create_wake_file(owner: u32) -> Result<(), InstallError> {
    let path = Path::new(ipc::WAKE_PATH);

    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.flush())
        .map_err(file_error("creating", path))?;
    chown(path, Some(owner), None).map_err(file_error("changing the owner of", path))?;

    fs::set_permissions(path, Permissions::from_mode(0o600))
        .map_err(file_error("changing the mode of", path))
}

fn spawn(program: &str, args: &[&str]) -> Result<Output, InstallError> {
    Command::new(program)
        .args(args)
        .output()
        .map_err(|source| InstallError::Spawn {
            command: program.to_owned(),
            source,
        })
}

fn run(program: &str, args: &[&str]) -> Result<Output, InstallError> {
    let output = spawn(program, args)?;

    if !output.status.success() {
        return Err(command_failed(program, args, &output));
    }

    Ok(output)
}

fn command_failed(program: &str, args: &[&str], output: &Output) -> InstallError {
    InstallError::CommandFailed {
        command: format!("{program} {}", args.join(" ")),
        status: output.status.to_string(),
        output: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> Owner {
        Owner {
            uid: 501,
            home: "/Users/alice".into(),
        }
    }

    #[test]
    fn install_copies_the_binaries_loads_the_daemon_and_autostarts_the_tray() {
        let plan = install_plan(
            Path::new("/Users/alice/Downloads/VyprLite"),
            &alice(),
            false,
        );

        assert_eq!(
            plan,
            [
                Step::Copy {
                    from: "/Users/alice/Downloads/VyprLite/vyprlite-helper".into(),
                    to: "/Library/PrivilegedHelperTools/com.vyprlite.helper".into(),
                },
                Step::Copy {
                    from: "/Users/alice/Downloads/VyprLite/VyprLite".into(),
                    to: "/usr/local/libexec/vyprlite/VyprLite".into(),
                },
                Step::Write {
                    path: "/Library/LaunchDaemons/com.vyprlite.helper.plist".into(),
                    contents: daemon_plist(501),
                    owner: 0,
                },
                Step::CreateWakeFile { owner: 501 },
                Step::Bootstrap,
                Step::Write {
                    path: "/Users/alice/Library/LaunchAgents/com.vyprlite.tray.plist".into(),
                    contents: AGENT_PLIST_TEXT.into(),
                    owner: 501,
                },
            ]
        );
    }

    #[test]
    fn install_unloads_a_loaded_daemon_before_loading_it_again() {
        let exe_dir = Path::new("/tmp/VyprLite");
        let mut expected = install_plan(exe_dir, &alice(), false);
        expected.insert(4, Step::Bootout);

        assert_eq!(install_plan(exe_dir, &alice(), true), expected);
    }

    #[test]
    fn daemon_plist_runs_the_installed_helper_for_the_owner() {
        let plist = daemon_plist(501);

        assert!(!plist.contains('@'), "{plist}");
        assert!(plist.contains(
            "<string>/Library/PrivilegedHelperTools/com.vyprlite.helper</string>\n\
             \t\t<string>--daemon</string>\n\
             \t\t<string>--owner-uid</string>\n\
             \t\t<string>501</string>"
        ));
        assert!(plist.contains(&format!("<string>{}</string>", ipc::WAKE_PATH)));
        assert!(plist.contains(&format!("<string>{DAEMON_LABEL}</string>")));
    }

    #[test]
    fn agent_plist_starts_the_installed_tray_app() {
        assert!(AGENT_PLIST_TEXT.contains(&format!("<string>{TRAY_PATH}</string>")));
    }

    #[test]
    fn uninstall_disconnects_a_running_helper_and_removes_everything() {
        assert_eq!(
            uninstall_plan(Path::new("/Users/alice"), true, |_| true),
            [
                Step::Disconnect,
                Step::Bootout,
                Step::Remove("/Users/alice/Library/LaunchAgents/com.vyprlite.tray.plist".into()),
                Step::Remove("/Library/LaunchDaemons/com.vyprlite.helper.plist".into()),
                Step::Remove("/var/db/vyprlite.wake".into()),
                Step::Remove("/var/run/vyprlite.sock".into()),
                Step::Remove("/Library/PrivilegedHelperTools/com.vyprlite.helper".into()),
                Step::RemoveDir("/usr/local/libexec/vyprlite".into()),
                Step::RemoveDir("/Library/Application Support/VyprLite".into()),
            ]
        );
    }

    #[test]
    fn uninstall_skips_the_disconnect_when_the_helper_is_not_running() {
        let plan = uninstall_plan(Path::new("/Users/alice"), true, |path| {
            path != Path::new(ipc::DAEMON_SOCKET_PATH)
        });

        assert_eq!(
            plan[..2],
            [
                Step::Bootout,
                Step::Remove("/Users/alice/Library/LaunchAgents/com.vyprlite.tray.plist".into())
            ]
        );
        assert!(!plan.contains(&Step::Disconnect));
    }

    #[test]
    fn uninstall_does_nothing_once_everything_is_gone() {
        assert_eq!(
            uninstall_plan(Path::new("/Users/alice"), false, |_| false),
            []
        );
    }

    #[test]
    fn parses_the_home_directory_from_dscl() {
        assert_eq!(
            parse_home("NFSHomeDirectory: /Users/alice\n"),
            Some(PathBuf::from("/Users/alice"))
        );
        assert_eq!(parse_home("No such key: NFSHomeDirectory\n"), None);
        assert_eq!(parse_home("NFSHomeDirectory: \n"), None);
    }
}
