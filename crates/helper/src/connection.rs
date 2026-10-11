use std::io;
use std::sync::Arc;

use ipc::{Method, Outcome, Reply, Request, Response, ServerMessage};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::task::{JoinError, JoinSet};

use crate::state::State;
use crate::vpn::Tunnel;

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("pipe I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("malformed message: {0}")]
    Codec(#[from] ipc::CodecError),
    #[error("answering a request failed: {0}")]
    Answer(#[from] JoinError),
}

/// Answers one tray app's requests and pushes every status and account change to it until it
/// disconnects. Requests are answered concurrently, so a slow VyprVPN call holds up nothing else.
pub async fn serve<S, T>(stream: S, state: Arc<State<T>>) -> Result<(), ConnectionError>
where
    S: AsyncRead + AsyncWrite,
    T: Tunnel,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();
    let mut status = state.status();
    let mut account = state.account().subscribe();
    let mut answers = JoinSet::new();

    loop {
        let message = tokio::select! {
            line = lines.next_line() => match line? {
                Some(line) => {
                    answers.spawn(answer(ipc::decode(&line)?, state.clone()));
                    continue;
                }
                None => return Ok(()),
            },
            Some(answered) = answers.join_next() => ServerMessage::Response(answered?),
            changed = status.changed() => match changed {
                Ok(()) => ServerMessage::Status(status.borrow_and_update().clone()),
                Err(_) => return Ok(()),
            },
            changed = account.changed() => match changed {
                Ok(()) => ServerMessage::Account(account.borrow_and_update().clone()),
                Err(_) => return Ok(()),
            },
        };

        writer.write_all(&ipc::encode(&message)?).await?;
    }
}

async fn answer<T: Tunnel>(request: Request, state: Arc<State<T>>) -> Response {
    let reply =
        match request.method {
            Method::Status => Ok(Reply::Status(state.status().borrow().clone())),
            Method::Account => Ok(Reply::Account(state.account().borrow().clone())),
            Method::Locations => state
                .locations()
                .await
                .map(Reply::Locations)
                .map_err(|error| format!("fetching the locations failed: {error}")),
            Method::Login(credentials) => state
                .login(credentials)
                .await
                .map(Reply::Account)
                .map_err(|error| {
                    log::warn!("logging in failed: {error}");
                    error.user_message()
                }),
            Method::Logout => state
                .logout()
                .map(Reply::Account)
                .map_err(|error| format!("logging out failed: {error}")),
            Method::Connect { spoke_id } => state
                .connect(spoke_id)
                .map(Reply::Status)
                .map_err(|error| error.to_string()),
            Method::Disconnect => Ok(Reply::Status(state.disconnect())),
        };

    let outcome = match reply {
        Ok(reply) => Outcome::Result(reply),
        Err(message) => {
            log::warn!("request {} failed: {message}", request.id);
            Outcome::Error(message)
        }
    };

    Response {
        id: request.id,
        outcome,
    }
}

#[cfg(test)]
mod tests {
    use ipc::{Account, Status};
    use tokio::io::{DuplexStream, ReadHalf, WriteHalf};
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::test_support::{LOGIN, SETTINGS, serve_one_session};
    use crate::vpn::fakes::{FakeTunnel, TUNNEL_IP};

    const LOCATIONS: &str = include_str!("../../vypr-api/tests/fixtures/locations.json");
    const CONNECT_DE1: &str = r#"{"id":8,"method":"connect","spoke_id":"de1"}"#;

    /// A helper with its own data directory, talking to a fake VyprVPN and a fake tunnel.
    struct Helper {
        tunnel: Arc<FakeTunnel>,
        vypr: MockServer,
        data_dir: tempfile::TempDir,
        state: Arc<State<Arc<FakeTunnel>>>,
    }

    impl Helper {
        async fn start() -> Self {
            let tunnel = Arc::new(FakeTunnel::default());
            let vypr = MockServer::start().await;
            let data_dir = tempfile::tempdir().unwrap();
            let state = Arc::new(State::new(&vypr.uri(), data_dir.path(), tunnel.clone()).unwrap());

            Self {
                tunnel,
                vypr,
                data_dir,
                state,
            }
        }

        fn restart(&mut self) {
            self.state = Arc::new(
                State::new(&self.vypr.uri(), self.data_dir.path(), self.tunnel.clone()).unwrap(),
            );
        }

        async fn wait_for(&self, settled: impl FnMut(&Status) -> bool) {
            self.state.status().wait_for(settled).await.unwrap();
        }

        fn connect(&self) -> TrayApp {
            let (tray_end, helper_end) = tokio::io::duplex(64 * 1024);
            tokio::spawn(serve(helper_end, self.state.clone()));

            let (reader, writer) = tokio::io::split(tray_end);
            TrayApp {
                lines: BufReader::new(reader).lines(),
                writer,
            }
        }
    }

    struct TrayApp {
        lines: tokio::io::Lines<BufReader<ReadHalf<DuplexStream>>>,
        writer: WriteHalf<DuplexStream>,
    }

    impl TrayApp {
        async fn send(&mut self, line: &str) {
            self.writer
                .write_all(format!("{line}\n").as_bytes())
                .await
                .unwrap();
        }

        async fn receive_line(&mut self) -> Option<String> {
            self.lines.next_line().await.unwrap()
        }

        async fn receive(&mut self) -> Option<ServerMessage> {
            let line = self.receive_line().await?;

            Some(ipc::decode(&line).unwrap())
        }

        async fn receive_status(&mut self) -> Status {
            loop {
                if let ServerMessage::Status(status) = self.receive().await.expect("helper hung up")
                {
                    return status;
                }
            }
        }

        async fn request(&mut self, line: &str) -> Outcome {
            self.send(line).await;

            loop {
                if let ServerMessage::Response(response) =
                    self.receive().await.expect("helper hung up")
                {
                    return response.outcome;
                }
            }
        }
    }

    fn logged_in() -> Outcome {
        Outcome::Result(Reply::Account(Account::LoggedIn {
            username: "alice".into(),
        }))
    }

    #[tokio::test]
    async fn answers_a_status_request() {
        let helper = Helper::start().await;
        let mut tray = helper.connect();

        tray.send(r#"{"id":42,"method":"status"}"#).await;

        assert_eq!(
            tray.receive().await,
            Some(ServerMessage::Response(Response {
                id: 42,
                outcome: Outcome::Result(Reply::Status(Status::Disconnected {
                    last_spoke_id: None,
                    error: None,
                })),
            }))
        );
    }

    #[tokio::test]
    async fn connects_and_pushes_the_status_to_every_tray_app() {
        let helper = Helper::start().await;
        serve_one_session(&helper.vypr).await;
        let mut tray = helper.connect();
        let mut other = helper.connect();
        tray.request(LOGIN).await;
        other.request(r#"{"id":1,"method":"status"}"#).await;

        assert_eq!(
            tray.request(CONNECT_DE1).await,
            Outcome::Result(Reply::Status(Status::Connecting {
                spoke_id: "de1".into()
            }))
        );

        loop {
            match other.receive_status().await {
                Status::Connecting { .. } => {}
                Status::Connected {
                    spoke_id,
                    tunnel_ip,
                    ..
                } => {
                    assert_eq!(spoke_id, "de1");
                    assert_eq!(tunnel_ip, TUNNEL_IP);
                    break;
                }
                status => panic!("{status:?}"),
            }
        }
        assert_eq!(helper.tunnel.live.lock().unwrap().len(), 1);

        tray.request(r#"{"id":9,"method":"disconnect"}"#).await;
        helper
            .wait_for(|status| matches!(status, Status::Disconnected { .. }))
            .await;
        assert!(helper.tunnel.live.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn refuses_to_connect_when_logged_out() {
        let helper = Helper::start().await;

        assert_eq!(
            helper.connect().request(CONNECT_DE1).await,
            Outcome::Error("not logged in".into())
        );
    }

    #[tokio::test]
    async fn logging_out_disconnects() {
        let helper = Helper::start().await;
        serve_one_session(&helper.vypr).await;
        let mut tray = helper.connect();
        tray.request(LOGIN).await;
        tray.request(CONNECT_DE1).await;
        helper
            .wait_for(|status| matches!(status, Status::Connected { .. }))
            .await;

        tray.request(r#"{"id":2,"method":"logout"}"#).await;

        helper
            .wait_for(|status| {
                *status
                    == Status::Disconnected {
                        last_spoke_id: Some("de1".into()),
                        error: None,
                    }
            })
            .await;
        assert!(helper.tunnel.live.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn hangs_up_on_a_malformed_request() {
        let helper = Helper::start().await;
        let mut tray = helper.connect();

        tray.send(r#"{"id":1,"method":"reboot"}"#).await;

        assert_eq!(tray.receive().await, None);
    }

    #[tokio::test]
    async fn fetches_the_locations_once() {
        let helper = Helper::start().await;
        Mock::given(path("/vyprvpn/locations-unauthenticated"))
            .respond_with(ResponseTemplate::new(200).set_body_string(LOCATIONS))
            .expect(1)
            .mount(&helper.vypr)
            .await;
        let mut tray = helper.connect();

        for id in 1..=2 {
            let outcome = tray
                .request(&format!(r#"{{"id":{id},"method":"locations"}}"#))
                .await;

            let Outcome::Result(Reply::Locations(locations)) = outcome else {
                panic!("{outcome:?}");
            };
            assert_eq!(locations.len(), 73);
        }
    }

    #[tokio::test]
    async fn retries_the_locations_after_a_failure() {
        let helper = Helper::start().await;
        Mock::given(path("/vyprvpn/locations-unauthenticated"))
            .respond_with(ResponseTemplate::new(503).set_body_string("maintenance"))
            .up_to_n_times(1)
            .mount(&helper.vypr)
            .await;
        Mock::given(path("/vyprvpn/locations-unauthenticated"))
            .respond_with(ResponseTemplate::new(200).set_body_string(LOCATIONS))
            .mount(&helper.vypr)
            .await;
        let mut tray = helper.connect();

        assert_eq!(
            tray.request(r#"{"id":1,"method":"locations"}"#).await,
            Outcome::Error(
                "fetching the locations failed: VyprVPN answered /vyprvpn/locations-unauthenticated \
                 with HTTP 503 Service Unavailable: maintenance"
                    .into()
            )
        );
        assert!(matches!(
            tray.request(r#"{"id":2,"method":"locations"}"#).await,
            Outcome::Result(Reply::Locations(_))
        ));
    }

    #[tokio::test]
    async fn maps_a_403_to_invalid_credentials() {
        let helper = Helper::start().await;
        Mock::given(path("/settings"))
            .respond_with(ResponseTemplate::new(403).set_body_string("Forbidden"))
            .mount(&helper.vypr)
            .await;
        let mut tray = helper.connect();

        assert_eq!(
            tray.request(LOGIN).await,
            Outcome::Error("invalid username or password".into())
        );
        assert_eq!(
            tray.request(r#"{"id":8,"method":"account"}"#).await,
            Outcome::Result(Reply::Account(Account::LoggedOut))
        );
    }

    #[tokio::test]
    async fn reports_login_failures_without_the_technical_detail() {
        let helper = Helper::start().await;
        Mock::given(path("/settings"))
            .respond_with(ResponseTemplate::new(500).set_body_string("<html>stack trace</html>"))
            .mount(&helper.vypr)
            .await;

        assert_eq!(
            helper.connect().request(LOGIN).await,
            Outcome::Error("VyprVPN returned an unexpected response.".into())
        );
    }

    #[tokio::test]
    async fn reports_an_unreachable_vyprvpn_plainly() {
        let mut helper = Helper::start().await;
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", listener.local_addr().unwrap())
        };
        helper.state =
            Arc::new(State::new(&closed, helper.data_dir.path(), helper.tunnel.clone()).unwrap());

        assert_eq!(
            helper.connect().request(LOGIN).await,
            Outcome::Error("Couldn't reach VyprVPN. Check your internet connection.".into())
        );
    }

    #[tokio::test]
    async fn keeps_the_login_across_restarts_until_logout() {
        let mut helper = Helper::start().await;
        Mock::given(path("/settings"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SETTINGS))
            .mount(&helper.vypr)
            .await;
        assert_eq!(helper.connect().request(LOGIN).await, logged_in());

        helper.restart();
        let mut restarted = helper.connect();
        assert_eq!(
            restarted.request(r#"{"id":1,"method":"account"}"#).await,
            logged_in()
        );

        assert_eq!(
            restarted.request(r#"{"id":2,"method":"logout"}"#).await,
            Outcome::Result(Reply::Account(Account::LoggedOut))
        );
        helper.restart();
        assert_eq!(
            helper
                .connect()
                .request(r#"{"id":3,"method":"account"}"#)
                .await,
            Outcome::Result(Reply::Account(Account::LoggedOut))
        );
    }

    #[tokio::test]
    async fn pushes_logins_to_every_tray_app_without_the_password() {
        let helper = Helper::start().await;
        Mock::given(path("/settings"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SETTINGS))
            .mount(&helper.vypr)
            .await;
        let mut tray = helper.connect();
        let mut other = helper.connect();
        other.request(r#"{"id":1,"method":"account"}"#).await;

        tray.send(LOGIN).await;
        let mut lines = vec![tray.receive_line().await.unwrap()];
        lines.push(tray.receive_line().await.unwrap());
        lines.push(other.receive_line().await.unwrap());

        assert!(
            lines.iter().all(|line| !line.contains("horse")),
            "{lines:?}"
        );
        assert_eq!(
            ipc::decode::<ServerMessage>(&lines[2]).unwrap(),
            ServerMessage::Account(Account::LoggedIn {
                username: "alice".into()
            })
        );
    }
}
