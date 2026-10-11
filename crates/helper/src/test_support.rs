use std::io;
use std::path::Path;
use std::sync::Arc;

use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::traits::tokio::Stream as _;
use interprocess::local_socket::{GenericFilePath, ToFsName};
use ipc::{Method, Request, ServerMessage};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::state::State;
use crate::vpn::fakes::FakeTunnel;

pub const SETTINGS: &str = include_str!("../../vypr-api/tests/fixtures/settings.json");
const CONNECT: &str = include_str!("../../vypr-api/tests/fixtures/connect.json");
pub const LOGIN: &str =
    r#"{"id":7,"method":"login","username":"alice","password":"correct horse"}"#;

pub type TestState = Arc<State<Arc<FakeTunnel>>>;

pub fn args(line: &str) -> impl Iterator<Item = String> {
    line.split_whitespace().map(str::to_owned)
}

/// Keep the directory alive as long as the state.
pub fn test_state(api_base_url: &str) -> (TestState, tempfile::TempDir) {
    let data_dir = tempfile::tempdir().unwrap();
    let state = State::new(
        api_base_url,
        data_dir.path(),
        Arc::new(FakeTunnel::default()),
    )
    .unwrap();

    (Arc::new(state), data_dir)
}

/// A VyprVPN that accepts one login and opens and deletes one session on de1.
pub async fn serve_one_session(vypr: &MockServer) {
    Mock::given(path("/settings"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SETTINGS))
        .expect(1)
        .mount(vypr)
        .await;
    Mock::given(method("POST"))
        .and(path("/vyprvpn/spokes/de1/wg/connections"))
        .respond_with(ResponseTemplate::new(200).set_body_string(CONNECT))
        .expect(1)
        .mount(vypr)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/vyprvpn/spokes/de1/wg/connections/48213977"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(vypr)
        .await;
}

/// Connects like the tray app does and asks for the status, keeping the connection open.
pub async fn request_status(socket: &Path) -> io::Result<(ServerMessage, Stream)> {
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
