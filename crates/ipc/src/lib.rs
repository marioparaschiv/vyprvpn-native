//! Messages exchanged between the tray app and the helper, and where to find the helper.
//!
//! The wire format is newline-delimited JSON; see `docs/ipc-protocol.md`.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::{DAEMON_SOCKET_PATH, WAKE_PATH, console_socket_path, current_user_uid};
#[cfg(windows)]
pub use windows::{
    APP_USER_MODEL_ID, SERVICE_NAME, SERVICE_PIPE_NAME, console_pipe_name, current_user_sid,
    process_user_sid, service_error_cause,
};

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

pub use serde_json::Error as CodecError;
pub use vypr_api::{Credentials, Location};

/// A request from the tray app. The helper answers each one with a [`Response`] carrying the same `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub method: Method,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Method {
    Status,
    Account,
    Locations,
    /// The only message that carries the password.
    Login(Credentials),
    Logout,
    Connect {
        spoke_id: String,
    },
    Disconnect,
}

/// Every line the helper sends: either the answer to a request or a pushed event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Response(Response),
    Status(Status),
    Account(Account),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Result(Reply),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Status(Status),
    Account(Account),
    Locations(Vec<Location>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Disconnected {
        last_spoke_id: Option<String>,
        /// Why the last attempt or connection failed; cleared on the next connect.
        error: Option<String>,
    },
    Connecting {
        spoke_id: String,
    },
    Connected {
        spoke_id: String,
        tunnel_ip: IpAddr,
        /// Unix seconds.
        connected_at: u64,
    },
    Disconnecting {
        spoke_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Account {
    LoggedOut,
    LoggedIn { username: String },
}

/// Serializes `message` as one protocol line, including the trailing newline.
pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, CodecError> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');

    Ok(line)
}

pub fn decode<'a, T: Deserialize<'a>>(line: &'a str) -> Result<T, CodecError> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_wire_format<T>(message: T, line: &str)
    where
        T: Serialize + for<'a> Deserialize<'a> + PartialEq + std::fmt::Debug,
    {
        assert_eq!(
            String::from_utf8(encode(&message).unwrap()).unwrap(),
            format!("{line}\n")
        );
        assert_eq!(decode::<T>(line).unwrap(), message);
    }

    #[test]
    fn status_request() {
        assert_wire_format(
            Request {
                id: 1,
                method: Method::Status,
            },
            r#"{"id":1,"method":"status"}"#,
        );
    }

    #[test]
    fn status_response() {
        assert_wire_format(
            ServerMessage::Response(Response {
                id: 1,
                outcome: Outcome::Result(Reply::Status(Status::Disconnected {
                    last_spoke_id: None,
                    error: None,
                })),
            }),
            r#"{"type":"response","id":1,"result":{"status":{"state":"disconnected","last_spoke_id":null,"error":null}}}"#,
        );
    }

    #[test]
    fn error_response() {
        assert_wire_format(
            ServerMessage::Response(Response {
                id: 7,
                outcome: Outcome::Error("not logged in".into()),
            }),
            r#"{"type":"response","id":7,"error":"not logged in"}"#,
        );
    }

    #[test]
    fn status_event() {
        assert_wire_format(
            ServerMessage::Status(Status::Disconnected {
                last_spoke_id: None,
                error: None,
            }),
            r#"{"type":"status","state":"disconnected","last_spoke_id":null,"error":null}"#,
        );
    }

    #[test]
    fn connect_request() {
        assert_wire_format(
            Request {
                id: 8,
                method: Method::Connect {
                    spoke_id: "de1".into(),
                },
            },
            r#"{"id":8,"method":"connect","spoke_id":"de1"}"#,
        );
    }

    #[test]
    fn statuses() {
        for (status, line) in [
            (
                Status::Disconnected {
                    last_spoke_id: Some("de1".into()),
                    error: Some("handshake timed out".into()),
                },
                r#"{"state":"disconnected","last_spoke_id":"de1","error":"handshake timed out"}"#,
            ),
            (
                Status::Connecting {
                    spoke_id: "de1".into(),
                },
                r#"{"state":"connecting","spoke_id":"de1"}"#,
            ),
            (
                Status::Connected {
                    spoke_id: "de1".into(),
                    tunnel_ip: [10, 1, 2, 3].into(),
                    connected_at: 1_760_140_800,
                },
                r#"{"state":"connected","spoke_id":"de1","tunnel_ip":"10.1.2.3","connected_at":1760140800}"#,
            ),
            (
                Status::Disconnecting {
                    spoke_id: "de1".into(),
                },
                r#"{"state":"disconnecting","spoke_id":"de1"}"#,
            ),
        ] {
            assert_wire_format(status, line);
        }
    }

    #[test]
    fn connect_response() {
        assert_wire_format(
            ServerMessage::Response(Response {
                id: 8,
                outcome: Outcome::Result(Reply::Status(Status::Connecting {
                    spoke_id: "de1".into(),
                })),
            }),
            r#"{"type":"response","id":8,"result":{"status":{"state":"connecting","spoke_id":"de1"}}}"#,
        );
    }

    #[test]
    fn locations_response() {
        assert_wire_format(
            ServerMessage::Response(Response {
                id: 2,
                outcome: Outcome::Result(Reply::Locations(vec![Location {
                    name: "Australia - Perth".into(),
                    country: "Australia".into(),
                    country_code: "AUS".into(),
                    region: "Oceania".into(),
                    spoke_id: "au3".into(),
                    hostname: "au3.vpn.goldenfrog.com".into(),
                }])),
            }),
            r#"{"type":"response","id":2,"result":{"locations":[{"name":"Australia - Perth","country":"Australia","country_code":"AUS","region":"Oceania","spoke_id":"au3","hostname":"au3.vpn.goldenfrog.com"}]}}"#,
        );
    }

    #[test]
    fn login_request() {
        assert_wire_format(
            Request {
                id: 3,
                method: Method::Login(Credentials {
                    username: "alice".into(),
                    password: "correct horse".into(),
                }),
            },
            r#"{"id":3,"method":"login","username":"alice","password":"correct horse"}"#,
        );
    }

    #[test]
    fn login_request_debug_hides_the_password() {
        let request = Request {
            id: 3,
            method: Method::Login(Credentials {
                username: "alice".into(),
                password: "correct horse".into(),
            }),
        };

        assert!(!format!("{request:?}").contains("horse"));
    }

    #[test]
    fn account_response_and_event() {
        assert_wire_format(
            ServerMessage::Response(Response {
                id: 4,
                outcome: Outcome::Result(Reply::Account(Account::LoggedIn {
                    username: "alice".into(),
                })),
            }),
            r#"{"type":"response","id":4,"result":{"account":{"state":"logged_in","username":"alice"}}}"#,
        );
        assert_wire_format(
            ServerMessage::Account(Account::LoggedOut),
            r#"{"type":"account","state":"logged_out"}"#,
        );
    }

    #[test]
    fn requests_without_fields() {
        for (method, name) in [
            (Method::Account, "account"),
            (Method::Locations, "locations"),
            (Method::Logout, "logout"),
            (Method::Disconnect, "disconnect"),
        ] {
            assert_wire_format(
                Request { id: 5, method },
                &format!(r#"{{"id":5,"method":"{name}"}}"#),
            );
        }
    }

    #[test]
    fn unknown_method_is_rejected() {
        assert!(decode::<Request>(r#"{"id":1,"method":"reboot"}"#).is_err());
    }
}
