mod credential_store;
mod install;
mod security;
mod service;
mod wireguard;

use std::env;
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use interprocess::local_socket::tokio::Listener;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName};
use interprocess::os::windows::local_socket::ListenerOptionsExt;

use crate::state::State;
use crate::{crash_log, server};
pub use credential_store::{CredentialStore, CredentialStoreError};
use security::{InvalidSid, Sid};
use wireguard::WireGuardNt;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Run(Mode),
    Install,
    Uninstall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// Foreground, unelevated, on a pipe only the current user can reach.
    Console,
    /// Started by the service control manager; `owner` is the user who installed VyprLite.
    Service { owner: Sid },
}

#[derive(Debug, thiserror::Error)]
enum HelperError {
    #[error("usage: vyprlite-helper install | uninstall | --console | --service --owner-sid <SID>")]
    Usage,
    #[error(transparent)]
    InvalidSid(#[from] InvalidSid),
    #[error("environment variable {0} is not set")]
    MissingEnv(&'static str),
    #[error("creating the data directory {path} failed: {source}")]
    DataDir { path: PathBuf, source: io::Error },
    #[error("opening the log file failed: {0}")]
    LogFile(io::Error),
    #[error("finding the current user failed: {0}")]
    CurrentUser(io::Error),
    #[error("starting the async runtime failed: {0}")]
    Runtime(io::Error),
    #[error("serving {pipe} failed: {source}")]
    Pipe { pipe: String, source: io::Error },
    #[error("waiting for Ctrl+C failed: {0}")]
    Signal(io::Error),
    #[error("talking to the service control manager failed: {0}")]
    Service(#[from] windows_service::Error),
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
    let data_dir = data_dir()?;
    crash_log::record_panics_in(data_dir.join("crash.log"));

    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));

    match mode {
        Mode::Console => {
            logger.init();
            run_console(&data_dir)
        }
        Mode::Service { owner } => {
            let log_file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(data_dir.join("helper.log"))
                .map_err(HelperError::LogFile)?;
            logger
                .target(env_logger::Target::Pipe(Box::new(log_file)))
                .init();
            service::run(owner, data_dir)
        }
    }
}

fn parse_command(mut args: impl Iterator<Item = String>) -> Result<Command, HelperError> {
    let mut next = || args.next();

    match (next().as_deref(), next().as_deref(), next(), next()) {
        (Some("install"), None, ..) => Ok(Command::Install),
        (Some("uninstall"), None, ..) => Ok(Command::Uninstall),
        (Some("--console"), None, ..) => Ok(Command::Run(Mode::Console)),
        (Some("--service"), Some("--owner-sid"), Some(owner), None) => {
            Ok(Command::Run(Mode::Service {
                owner: Sid::parse(&owner)?,
            }))
        }
        _ => Err(HelperError::Usage),
    }
}

/// For the LocalSystem service this is the system profile's local app data, which only SYSTEM and
/// Administrators can open; a console helper gets the current user's.
fn data_dir() -> Result<PathBuf, HelperError> {
    let path =
        PathBuf::from(env::var_os("LOCALAPPDATA").ok_or(HelperError::MissingEnv("LOCALAPPDATA"))?)
            .join("VyprLite");

    fs::create_dir_all(&path).map_err(|source| HelperError::DataDir {
        path: path.clone(),
        source,
    })?;

    Ok(path)
}

fn run_console(data_dir: &Path) -> Result<(), HelperError> {
    let owner = Sid::parse(&ipc::current_user_sid().map_err(HelperError::CurrentUser)?)?;
    let pipe = ipc::console_pipe_name(&owner.to_string());
    log::info!("serving {pipe}; press Ctrl+C to stop");

    serve(&pipe, &owner, data_dir, async {
        tokio::signal::ctrl_c().await.map_err(HelperError::Signal)
    })
}

/// Serves the helper on `pipe`, reachable only by SYSTEM and `owner`, until `shutdown` completes or
/// neither a tray app nor a tunnel has been up for a while.
fn serve(
    pipe: &str,
    owner: &Sid,
    data_dir: &Path,
    shutdown: impl Future<Output = Result<(), HelperError>>,
) -> Result<(), HelperError> {
    let runtime = tokio::runtime::Runtime::new().map_err(HelperError::Runtime)?;

    runtime.block_on(async {
        let listener = listen(pipe, owner).map_err(|source| HelperError::Pipe {
            pipe: pipe.to_owned(),
            source,
        })?;
        let state = State::new(vypr_api::DEFAULT_BASE_URL, data_dir, WireGuardNt)?;

        tokio::select! {
            served = server::serve(listener, Arc::new(state), server::IDLE_EXIT_AFTER, |_| Ok(())) => served.map_err(|source| HelperError::Pipe {
                pipe: pipe.to_owned(),
                source,
            }),
            result = shutdown => result,
        }
    })
}

/// Fails if anyone else already serves `pipe`, so the name cannot be squatted while the helper runs.
fn listen(pipe: &str, owner: &Sid) -> io::Result<Listener> {
    ListenerOptions::new()
        .name(pipe.to_fs_name::<GenericFilePath>()?)
        .security_descriptor(security::pipe_security(owner)?)
        .create_tokio()
}

#[cfg(test)]
mod tests {
    use interprocess::local_socket::tokio::Stream;
    use interprocess::local_socket::traits::tokio::Stream as _;
    use ipc::{Method, Outcome, Reply, Request, Response, ServerMessage, Status};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::vpn::fakes::FakeTunnel;

    type TestState = Arc<State<Arc<FakeTunnel>>>;

    fn args(line: &str) -> impl Iterator<Item = String> {
        line.split_whitespace().map(str::to_owned)
    }

    /// Keep the directory alive as long as the state.
    fn test_state(api_base_url: &str) -> (TestState, tempfile::TempDir) {
        let data_dir = tempfile::tempdir().unwrap();
        let state = State::new(
            api_base_url,
            data_dir.path(),
            Arc::new(FakeTunnel::default()),
        )
        .unwrap();

        (Arc::new(state), data_dir)
    }

    fn test_pipe_name(test: &str) -> String {
        format!(
            r"\\.\pipe\vyprlite-helper-test-{test}-{}",
            std::process::id()
        )
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
            parse_command(args("--service --owner-sid S-1-5-21-1-2-3-1001")).unwrap(),
            Command::Run(Mode::Service {
                owner: Sid::parse("S-1-5-21-1-2-3-1001").unwrap()
            })
        );
    }

    #[test]
    fn rejects_bad_arguments() {
        for line in [
            "",
            "--service",
            "--service --owner-sid",
            "--console extra",
            "--verbose",
            "install now",
            "uninstall --console",
        ] {
            assert!(
                matches!(parse_command(args(line)), Err(HelperError::Usage)),
                "{line}"
            );
        }
        assert!(matches!(
            parse_command(args("--service --owner-sid WD")),
            Err(HelperError::InvalidSid(_))
        ));
    }

    /// Connects like the tray app does and asks for the status, keeping the connection open.
    async fn request_status(pipe: &str) -> io::Result<(ServerMessage, Stream)> {
        let mut stream = Stream::connect(pipe.to_fs_name::<GenericFilePath>()?).await?;
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
    async fn owner_gets_status_over_the_pipe() {
        let pipe = test_pipe_name("owner");
        let owner = Sid::parse(&ipc::current_user_sid().unwrap()).unwrap();
        let listener = listen(&pipe, &owner).unwrap();
        let (state, _data_dir) = test_state(vypr_api::DEFAULT_BASE_URL);

        let reply = tokio::select! {
            served = server::serve(listener, state, server::IDLE_EXIT_AFTER, |_| Ok(())) => panic!("helper stopped: {served:?}"),
            reply = request_status(&pipe) => reply.unwrap().0,
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
    async fn other_users_are_denied() {
        let pipe = test_pipe_name("other");
        let stranger = Sid::parse("S-1-5-21-1-2-3-1001").unwrap();
        let _listener = listen(&pipe, &stranger).unwrap();

        let error = request_status(&pipe).await.unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn refuses_a_pipe_name_someone_else_serves() {
        let pipe = test_pipe_name("squatted");
        let owner = Sid::parse(&ipc::current_user_sid().unwrap()).unwrap();
        let _squatter = listen(&pipe, &owner).unwrap();

        assert!(listen(&pipe, &owner).is_err());
    }

    #[tokio::test]
    async fn exits_once_no_tray_app_is_connected() {
        let pipe = test_pipe_name("idle");
        let owner = Sid::parse(&ipc::current_user_sid().unwrap()).unwrap();
        let listener = listen(&pipe, &owner).unwrap();
        let (state, _data_dir) = test_state(vypr_api::DEFAULT_BASE_URL);
        let helper = tokio::spawn(server::serve(
            listener,
            state,
            server::IDLE_EXIT_AFTER,
            |_| Ok(()),
        ));
        let (_, tray_app) = request_status(&pipe).await.unwrap();

        tokio::time::pause();
        tokio::time::advance(server::IDLE_EXIT_AFTER * 2).await;
        assert!(!helper.is_finished());

        drop(tray_app);
        helper.await.unwrap().unwrap();
    }

    async fn request(tray: &mut BufReader<Stream>, request: &str) -> Outcome {
        tray.write_all(
            format!(
                "{request}
"
            )
            .as_bytes(),
        )
        .await
        .unwrap();

        loop {
            let mut line = String::new();
            tray.read_line(&mut line).await.unwrap();
            if let ServerMessage::Response(response) = ipc::decode(&line).unwrap() {
                return response.outcome;
            }
        }
    }

    #[tokio::test]
    async fn connects_over_the_pipe_and_stays_up_while_the_tunnel_is() {
        let vypr = MockServer::start().await;
        Mock::given(path("/settings"))
            .respond_with(ResponseTemplate::new(200).set_body_string(include_str!(
                "../../../vypr-api/tests/fixtures/settings.json"
            )))
            .mount(&vypr)
            .await;
        Mock::given(method("POST"))
            .and(path("/vyprvpn/spokes/de1/wg/connections"))
            .respond_with(ResponseTemplate::new(200).set_body_string(include_str!(
                "../../../vypr-api/tests/fixtures/connect.json"
            )))
            .expect(1)
            .mount(&vypr)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/vyprvpn/spokes/de1/wg/connections/48213977"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&vypr)
            .await;
        let pipe = test_pipe_name("connect");
        let owner = Sid::parse(&ipc::current_user_sid().unwrap()).unwrap();
        let listener = listen(&pipe, &owner).unwrap();
        let (state, _data_dir) = test_state(&vypr.uri());
        let helper = tokio::spawn(server::serve(
            listener,
            state.clone(),
            server::IDLE_EXIT_AFTER,
            |_| Ok(()),
        ));
        let mut tray = BufReader::new(
            Stream::connect(pipe.to_fs_name::<GenericFilePath>().unwrap())
                .await
                .unwrap(),
        );

        request(
            &mut tray,
            r#"{"id":1,"method":"login","username":"alice","password":"correct horse"}"#,
        )
        .await;
        assert_eq!(
            request(&mut tray, r#"{"id":2,"method":"connect","spoke_id":"de1"}"#).await,
            Outcome::Result(Reply::Status(Status::Connecting {
                spoke_id: "de1".into()
            }))
        );
        state
            .status()
            .wait_for(|status| matches!(status, Status::Connected { .. }))
            .await
            .unwrap();

        drop(tray);
        tokio::time::pause();
        tokio::time::advance(server::IDLE_EXIT_AFTER * 2).await;
        assert!(!helper.is_finished());
        tokio::time::resume();

        state.disconnect();
        state
            .status()
            .wait_for(|status| matches!(status, Status::Disconnected { .. }))
            .await
            .unwrap();
        tokio::time::pause();
        helper.await.unwrap().unwrap();
    }
}
