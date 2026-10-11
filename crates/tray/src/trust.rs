use interprocess::local_socket::PeerCreds;

use crate::helper_link::Helper;
#[cfg(windows)]
use crate::service;

/// Why the process serving the helper's pipe is not trusted with requests.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Untrusted {
    #[cfg(windows)]
    #[error("the helper pipe is served by process {server}, not by the helper service ({})", service_label(*.service))]
    NotTheService { server: u32, service: Option<u32> },
    #[cfg(windows)]
    #[error("the helper pipe is served by process {server} running as {actual}, not as {expected}")]
    WrongUser {
        server: u32,
        expected: String,
        actual: String,
    },
    #[cfg(target_os = "macos")]
    #[error("the helper socket is served by uid {actual}, not by root{}", console_user_label(*.console_user))]
    WrongUid {
        actual: u32,
        console_user: Option<u32>,
    },
    #[error("cannot tell who serves the helper pipe: {0}")]
    Unverifiable(String),
}

#[cfg(windows)]
pub fn verify_peer(helper: &Helper, peer: &PeerCreds) -> Result<(), Untrusted> {
    let server = peer
        .pid()
        .ok_or_else(|| Untrusted::Unverifiable("the pipe server's process is unknown".into()))?;

    verify_server(helper, server)
}

/// The helper usually is not running, so any local user could create its socket first and collect
/// the password. Only root, or for `--console` this same user, may be sent anything. macOS reports
/// the peer's uid but not its process.
#[cfg(target_os = "macos")]
pub fn verify_peer(helper: &Helper, peer: &PeerCreds) -> Result<(), Untrusted> {
    let actual = peer
        .euid()
        .ok_or_else(|| Untrusted::Unverifiable("the socket server's user is unknown".into()))?;
    let console_user = match helper {
        Helper::Service => None,
        Helper::Console { .. } => Some(ipc::current_user_uid()),
    };

    check_uid(actual, console_user)
}

/// The helper usually is not running, so any local process could create its pipe first and collect
/// the password. Only the service process the Service Control Manager started, or for `--console`
/// a process of this same user, may be sent anything.
#[cfg(windows)]
fn verify_server(helper: &Helper, server: u32) -> Result<(), Untrusted> {
    match helper {
        Helper::Service => {
            let service = service::process_id()
                .map_err(|error| Untrusted::Unverifiable(error.to_string()))?;
            check_service(server, service)
        }
        Helper::Console { user_sid, .. } => {
            let actual = ipc::process_user_sid(server)
                .map_err(|error| Untrusted::Unverifiable(error.to_string()))?;
            check_user(server, user_sid, &actual)
        }
    }
}

#[cfg(windows)]
fn check_service(server: u32, service: Option<u32>) -> Result<(), Untrusted> {
    if service != Some(server) {
        return Err(Untrusted::NotTheService { server, service });
    }

    Ok(())
}

#[cfg(windows)]
fn check_user(server: u32, expected: &str, actual: &str) -> Result<(), Untrusted> {
    if expected != actual {
        return Err(Untrusted::WrongUser {
            server,
            expected: expected.to_owned(),
            actual: actual.to_owned(),
        });
    }

    Ok(())
}

#[cfg(windows)]
fn service_label(service: Option<u32>) -> String {
    match service {
        Some(process_id) => format!("process {process_id}"),
        None => "not running".to_owned(),
    }
}

#[cfg(target_os = "macos")]
const ROOT_UID: u32 = 0;

#[cfg(target_os = "macos")]
fn check_uid(actual: u32, console_user: Option<u32>) -> Result<(), Untrusted> {
    if actual != ROOT_UID && Some(actual) != console_user {
        return Err(Untrusted::WrongUid {
            actual,
            console_user,
        });
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn console_user_label(console_user: Option<u32>) -> String {
    match console_user {
        Some(uid) => format!(" or uid {uid}"),
        None => String::new(),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;

    #[test]
    fn trusts_root_and_in_console_mode_this_user() {
        assert_eq!(check_uid(ROOT_UID, None), Ok(()));
        assert_eq!(check_uid(ROOT_UID, Some(501)), Ok(()));
        assert_eq!(check_uid(501, Some(501)), Ok(()));
    }

    #[test]
    fn distrusts_any_other_user() {
        assert_eq!(
            check_uid(502, Some(501)).unwrap_err().to_string(),
            "the helper socket is served by uid 502, not by root or uid 501"
        );
        assert_eq!(
            check_uid(501, None).unwrap_err().to_string(),
            "the helper socket is served by uid 501, not by root"
        );
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    const USER: &str = "S-1-5-21-1-2-3-1001";
    const STRANGER: &str = "S-1-5-21-1-2-3-1002";

    #[test]
    fn trusts_only_the_service_process() {
        assert_eq!(check_service(812, Some(812)), Ok(()));
        assert_eq!(
            check_service(4242, Some(812)),
            Err(Untrusted::NotTheService {
                server: 4242,
                service: Some(812)
            })
        );
    }

    #[test]
    fn distrusts_a_pipe_served_while_the_service_is_stopped() {
        let squatted = check_service(4242, None).unwrap_err();

        assert_eq!(
            squatted.to_string(),
            "the helper pipe is served by process 4242, not by the helper service (not running)"
        );
    }

    #[test]
    fn trusts_only_the_same_user_in_console_mode() {
        assert_eq!(check_user(77, USER, USER), Ok(()));
        assert_eq!(
            check_user(77, USER, STRANGER),
            Err(Untrusted::WrongUser {
                server: 77,
                expected: USER.into(),
                actual: STRANGER.into(),
            })
        );
    }

    #[test]
    fn verifies_this_process_as_its_own_user() {
        let helper = Helper::Console {
            pipe_name: std::ffi::OsString::new(),
            user_sid: ipc::current_user_sid().unwrap(),
        };

        assert_eq!(verify_server(&helper, std::process::id()), Ok(()));
    }
}
