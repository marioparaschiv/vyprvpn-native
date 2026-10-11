mod boringtun;
mod credential_store;
mod install;
mod teardown_store;

use std::env;
use std::fs::{self, DirBuilder, OpenOptions, Permissions};
use std::future::Future;
use std::io;
use std::num::ParseIntError;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::StreamCommon as _;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName};
use tokio::signal::unix::{SignalKind, signal};

use crate::state::State;
use crate::{crash_log, server};
use boringtun::Boringtun;
pub use credential_store::{CredentialStore, CredentialStoreError};
pub use teardown_store::{TeardownStore, TeardownStoreError};

const DAEMON_DATA_DIR: &str = "/Library/Application Support/VyprLite";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Run(Mode),
    Install,
    Uninstall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// Foreground, as the current user, on a socket in their private temporary directory.
    Console,
    /// Started by launchd as root; `owner` is the uid of the user who installed VyprLite.
    Daemon { owner: u32 },
}

#[derive(Debug, thiserror::Error)]
enum HelperError {
    #[error("usage: vyprlite-helper install | uninstall | --console | --daemon --owner-uid <UID>")]
    Usage,
    #[error("the owner uid is not a number: {0}")]
    InvalidUid(#[from] ParseIntError),
    #[error("environment variable {0} is not set")]
    MissingEnv(&'static str),
    #[error("creating the data directory {path} failed: {source}")]
    DataDir { path: PathBuf, source: io::Error },
    #[error("opening the log file failed: {0}")]
    LogFile(io::Error),
    #[error("starting the async runtime failed: {0}")]
    Runtime(io::Error),
    #[error("serving {path} failed: {source}")]
    Socket { path: PathBuf, source: io::Error },
    #[error("waiting for a signal failed: {0}")]
    Signal(io::Error),
    #[error("setting up the VyprVPN client failed: {0}")]
    Api(#[from] vypr_api::Error),
    #[error(transparent)]
    Install(#[from] install::InstallError),
}

pub fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), HelperError> {
    let mode = match parse_command(env::args().skip(1))? {
        Command::Run(mode) => mode,
        Command::Install => return Ok(install::install()?),
        Command::Uninstall => return Ok(install::uninstall()?),
    };
    let data_dir = data_dir(&mode)?;
    crash_log::record_panics_in(data_dir.join("crash.log"));

    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));

    match mode {
        Mode::Console => {
            logger.init();
            let socket = ipc::console_socket_path();
            log::info!("serving {}; press Ctrl+C to stop", socket.display());

            serve(&socket, ipc::current_user_uid(), &data_dir, async {
                tokio::signal::ctrl_c().await.map_err(HelperError::Signal)
            })
        }
        Mode::Daemon { owner } => {
            let log_file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(data_dir.join("helper.log"))
                .map_err(HelperError::LogFile)?;
            logger
                .target(env_logger::Target::Pipe(Box::new(log_file)))
                .init();

            serve(
                Path::new(ipc::DAEMON_SOCKET_PATH),
                owner,
                &data_dir,
                async {
                    signal(SignalKind::terminate())
                        .map_err(HelperError::Signal)?
                        .recv()
                        .await;
                    Ok(())
                },
            )
        }
    }
}

fn parse_command(mut args: impl Iterator<Item = String>) -> Result<Command, HelperError> {
    let mut next = || args.next();

    match (next().as_deref(), next().as_deref(), next(), next()) {
        (Some("install"), None, ..) => Ok(Command::Install),
        (Some("uninstall"), None, ..) => Ok(Command::Uninstall),
        (Some("--console"), None, ..) => Ok(Command::Run(Mode::Console)),
        (Some("--daemon"), Some("--owner-uid"), Some(owner), None) => {
            Ok(Command::Run(Mode::Daemon {
                owner: owner.parse()?,
            }))
        }
        _ => Err(HelperError::Usage),
    }
}

/// Mode 0700 either way: root's for the daemon, the current user's for a console helper.
fn data_dir(mode: &Mode) -> Result<PathBuf, HelperError> {
    let path = match mode {
        Mode::Daemon { .. } => PathBuf::from(DAEMON_DATA_DIR),
        Mode::Console => PathBuf::from(env::var_os("HOME").ok_or(HelperError::MissingEnv("HOME"))?)
            .join("Library/Application Support/VyprLite"),
    };

    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&path)
        .and_then(|()| fs::set_permissions(&path, Permissions::from_mode(0o700)))
        .map_err(|source| HelperError::DataDir {
            path: path.clone(),
            source,
        })?;

    Ok(path)
}

/// Serves the helper on `socket`, reachable only by root and `owner`, until `shutdown` completes
/// or no tray app has been connected for a while.
fn serve(
    socket: &Path,
    owner: u32,
    data_dir: &Path,
    shutdown: impl Future<Output = Result<(), HelperError>>,
) -> Result<(), HelperError> {
    let runtime = tokio::runtime::Runtime::new().map_err(HelperError::Runtime)?;
    let socket_error = |source| HelperError::Socket {
        path: socket.to_owned(),
        source,
    };

    runtime.block_on(async {
        let listener = listen(socket, owner).map_err(socket_error)?;
        let state = State::new(
            vypr_api::DEFAULT_BASE_URL,
            data_dir,
            Boringtun::new(data_dir),
        )?;

        tokio::select! {
            served = server::serve(listener, Arc::new(state), server::IDLE_EXIT_AFTER, |stream| admit(stream, owner)) => served.map_err(socket_error),
            result = shutdown => result,
        }
    })
}

/// Replaces a socket left behind by a helper that crashed; only root can write the daemon's
/// directory and only the user their temporary one, so nobody else can have put it there.
fn listen(socket: &Path, owner: u32) -> io::Result<Listener> {
    let listener = ListenerOptions::new()
        .name(socket.to_fs_name::<GenericFilePath>()?)
        .try_overwrite(true)
        .create_tokio()?;

    // Connecting to a Unix socket on macOS needs write access to it, so this keeps everyone but
    // root and the owner out before the peer check below sees them.
    std::os::unix::fs::chown(socket, Some(owner), None)?;
    fs::set_permissions(socket, Permissions::from_mode(0o600))?;

    Ok(listener)
}

fn admit(stream: &Stream, owner: u32) -> io::Result<()> {
    let uid = stream
        .peer_creds()?
        .euid()
        .ok_or_else(|| io::Error::other("the peer's uid is unknown"))?;

    if admits(uid, owner) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("uid {uid} may not use the helper"),
        ))
    }
}

fn admits(uid: u32, owner: u32) -> bool {
    uid == 0 || uid == owner
}

#[cfg(test)]
mod tests {
    use interprocess::local_socket::traits::tokio::Stream as _;
    use ipc::{Method, Outcome, Reply, Request, Response, ServerMessage, Status};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;
    use crate::vpn::fakes::FakeTunnel;

    type TestState = Arc<State<Arc<FakeTunnel>>>;

    fn args(line: &str) -> impl Iterator<Item = String> {
        line.split_whitespace().map(str::to_owned)
    }

    /// Keep the directory alive as long as the state.
    fn test_state() -> (TestState, tempfile::TempDir) {
        let data_dir = tempfile::tempdir().unwrap();
        let state = State::new(
            vypr_api::DEFAULT_BASE_URL,
            data_dir.path(),
            Arc::new(FakeTunnel::default()),
        )
        .unwrap();

        (Arc::new(state), data_dir)
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse_command(args("install")).unwrap(), Command::Install);
        assert_eq!(
            parse_command(args("uninstall")).unwrap(),
            Command::Uninstall
        );
        assert_eq!(
            parse_command(args("--console")).unwrap(),
            Command::Run(Mode::Console)
        );
        assert_eq!(
            parse_command(args("--daemon --owner-uid 501")).unwrap(),
            Command::Run(Mode::Daemon { owner: 501 })
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        for line in [
            "",
            "--daemon",
            "--daemon --owner-uid",
            "--console extra",
            "install --force",
            "--verbose",
        ] {
            assert!(
                matches!(parse_command(args(line)), Err(HelperError::Usage)),
                "{line}"
            );
        }
        assert!(matches!(
            parse_command(args("--daemon --owner-uid alice")),
            Err(HelperError::InvalidUid(_))
        ));
    }

    #[test]
    fn admits_only_root_and_the_owner() {
        assert!(admits(0, 501));
        assert!(admits(501, 501));
        assert!(!admits(502, 501));
    }

    /// Connects like the tray app does and asks for the status, keeping the connection open.
    async fn request_status(socket: &Path) -> io::Result<(ServerMessage, Stream)> {
        let mut stream = Stream::connect(socket.to_fs_name::<GenericFilePath>()?).await?;
        let request = Request {
            id: 3,
            method: Method::Status,
        };
        stream.write_all(&ipc::encode(&request)?).await?;

        let mut line = String::new();
        BufReader::new(&mut stream).read_line(&mut line).await?;

        Ok((ipc::decode(&line)?, stream))
    }

    #[tokio::test]
    async fn owner_gets_status_over_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("helper.sock");
        let owner = ipc::current_user_uid();
        let listener = listen(&socket, owner).unwrap();
        let (state, _data_dir) = test_state();

        let reply = tokio::select! {
            served = server::serve(listener, state, server::IDLE_EXIT_AFTER, |stream| admit(stream, owner)) => panic!("helper stopped: {served:?}"),
            reply = request_status(&socket) => reply.unwrap().0,
        };

        assert_eq!(
            reply,
            ServerMessage::Response(Response {
                id: 3,
                outcome: Outcome::Result(Reply::Status(Status::Disconnected {
                    last_spoke_id: None,
                    error: None,
                })),
            })
        );
    }

    #[tokio::test]
    async fn only_the_owner_can_open_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("helper.sock");
        let _listener = listen(&socket, ipc::current_user_uid()).unwrap();

        let mode = fs::metadata(&socket).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[tokio::test]
    async fn replaces_a_socket_left_by_a_crashed_helper() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("helper.sock");
        let stale = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        drop(stale);

        listen(&socket, ipc::current_user_uid()).unwrap();
    }
}
