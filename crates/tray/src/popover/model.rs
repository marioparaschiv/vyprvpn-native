//! What the popover shows, as pure functions of the [`LinkState`].

use std::net::IpAddr;
use std::time::{Duration, SystemTime};

use ipc::{Account, Location, Status};

use crate::helper_link::{HelperState, LinkState, Locations};

/// The full-width button under the location row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectButton {
    pub label: String,
    /// `None` disables the button.
    pub action: Option<ConnectAction>,
    /// Shows a spinner while the helper is connecting or disconnecting.
    pub busy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectAction {
    Connect(String),
    Disconnect,
}

pub fn connect_button(state: &LinkState) -> ConnectButton {
    let logged_in = matches!(state.account, Some(Account::LoggedIn { .. }));
    let busy = match &state.helper {
        HelperState::Running(Status::Connecting { .. }) if logged_in => Some("Connecting…"),
        HelperState::Running(Status::Disconnecting { .. }) if logged_in => Some("Disconnecting…"),
        _ => None,
    };
    if let Some(label) = busy {
        return ConnectButton {
            label: label.to_owned(),
            action: None,
            busy: true,
        };
    }

    let (label, action) = match (state.connect_target(), state.can_disconnect()) {
        (Some(target), true) => (
            format!("Switch to {}", state.locations.name(target)),
            Some(ConnectAction::Connect(target.to_owned())),
        ),
        (Some(target), false) => (
            "Connect".to_owned(),
            Some(ConnectAction::Connect(target.to_owned())),
        ),
        (None, true) => ("Disconnect".to_owned(), Some(ConnectAction::Disconnect)),
        (None, false) => ("Connect".to_owned(), None),
    };

    ConnectButton {
        label,
        action,
        busy: false,
    }
}

/// The colour family of the status indicator; the headline always says the same in words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Idle,
    Busy,
    Protected,
    Failed,
}

/// The status zone at the top of the main page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headline {
    pub tone: Tone,
    pub title: String,
    pub detail: Detail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    /// A line of supporting text.
    Text(String),
    Connected {
        spoke_id: String,
        tunnel_ip: IpAddr,
        /// Unix seconds.
        connected_at: u64,
    },
}

pub fn headline(state: &LinkState) -> Headline {
    let headline = |tone, title: &str, detail: String| Headline {
        tone,
        title: title.to_owned(),
        detail: Detail::Text(detail),
    };

    let status = match &state.helper {
        HelperState::Running(status) => status,
        HelperState::NotRunning => {
            return headline(
                Tone::Failed,
                "Helper not running",
                "Waiting for the VyprVPN helper.".to_owned(),
            );
        }
        HelperState::StartFailed(reason) => {
            return headline(Tone::Failed, "Helper not running", reason.clone());
        }
        HelperState::Untrusted(untrusted) => {
            return headline(Tone::Failed, "Helper not trusted", untrusted.to_string());
        }
    };

    match status {
        Status::Disconnected {
            error: Some(error), ..
        } => headline(Tone::Failed, "Connection failed", error.clone()),
        Status::Disconnected { error: None, .. } => headline(
            Tone::Idle,
            "Disconnected",
            match state.account {
                Some(Account::LoggedOut) => "Log in from the tray menu to connect.".to_owned(),
                Some(Account::LoggedIn { .. }) | None => {
                    "Your traffic is not protected.".to_owned()
                }
            },
        ),
        Status::Connecting { spoke_id } => headline(
            Tone::Busy,
            "Connecting…",
            state.locations.name(spoke_id).to_owned(),
        ),
        Status::Disconnecting { spoke_id } => headline(
            Tone::Busy,
            "Disconnecting…",
            state.locations.name(spoke_id).to_owned(),
        ),
        Status::Connected {
            spoke_id,
            tunnel_ip,
            connected_at,
        } => Headline {
            tone: Tone::Protected,
            title: "Secured".to_owned(),
            detail: Detail::Connected {
                spoke_id: spoke_id.clone(),
                tunnel_ip: *tunnel_ip,
                connected_at: *connected_at,
            },
        },
    }
}

/// The location row above the button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationRow<'a> {
    Loading,
    Failed(&'a str),
    Empty,
    Selected(&'a Location),
}

pub fn location_row(state: &LinkState) -> LocationRow<'_> {
    match &state.locations {
        Locations::Loading => LocationRow::Loading,
        Locations::Failed(reason) => LocationRow::Failed(reason),
        Locations::Loaded(_) => state
            .selected_spoke_id()
            .and_then(|spoke_id| state.locations.find(spoke_id))
            .map_or(LocationRow::Empty, LocationRow::Selected),
    }
}

/// How long the tunnel has been up, as `m:ss` or `h:mm:ss`. A clock behind `connected_at` reads
/// as just connected.
pub fn connected_for(connected_at: u64, now: SystemTime) -> String {
    let since = SystemTime::UNIX_EPOCH + Duration::from_secs(connected_at);
    let seconds = now.duration_since(since).unwrap_or_default().as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub fn location(name: &str, country: &str, code: &str, region: &str, spoke: &str) -> Location {
        Location {
            name: name.into(),
            country: country.into(),
            country_code: code.into(),
            region: region.into(),
            spoke_id: spoke.into(),
            hostname: format!("{spoke}.vpn.goldenfrog.com"),
        }
    }

    pub fn locations() -> Vec<Location> {
        vec![
            location("Germany", "Germany", "GER", "Europe", "de1"),
            location(
                "Australia - Perth",
                "Australia",
                "AUS",
                "Asia Pacific",
                "au3",
            ),
            location(
                "Australia - Sydney",
                "Australia",
                "AUS",
                "Asia Pacific",
                "au1",
            ),
            location("USA - Miami", "United States", "USA", "Americas", "us5"),
        ]
    }

    pub fn state(status: Status, selected: Option<&str>) -> LinkState {
        LinkState {
            helper: HelperState::Running(status),
            account: Some(Account::LoggedIn {
                username: "alice".into(),
            }),
            locations: Locations::Loaded(locations()),
            selected: selected.map(Into::into),
        }
    }

    fn disconnected(last_spoke_id: Option<&str>, error: Option<&str>) -> Status {
        Status::Disconnected {
            last_spoke_id: last_spoke_id.map(Into::into),
            error: error.map(Into::into),
        }
    }

    fn connected(spoke_id: &str) -> Status {
        Status::Connected {
            spoke_id: spoke_id.into(),
            tunnel_ip: [10, 1, 2, 3].into(),
            connected_at: 1_760_140_800,
        }
    }

    fn enabled(label: &str, action: ConnectAction) -> ConnectButton {
        ConnectButton {
            label: label.into(),
            action: Some(action),
            busy: false,
        }
    }

    fn disabled(label: &str, busy: bool) -> ConnectButton {
        ConnectButton {
            label: label.into(),
            action: None,
            busy,
        }
    }

    fn connect(spoke_id: &str) -> ConnectAction {
        ConnectAction::Connect(spoke_id.into())
    }

    #[test]
    fn disconnected_connects_to_the_selection_else_the_last_used_else_the_first() {
        let cases = [
            (state(disconnected(Some("us5"), None), Some("au3")), "au3"),
            (state(disconnected(Some("us5"), None), None), "us5"),
            (state(disconnected(None, None), None), "de1"),
            (
                state(disconnected(None, Some("handshake timed out")), None),
                "de1",
            ),
        ];

        for (state, spoke_id) in cases {
            assert_eq!(
                connect_button(&state),
                enabled("Connect", connect(spoke_id))
            );
        }
    }

    #[test]
    fn disconnected_without_any_location_cannot_connect() {
        let mut state = state(disconnected(None, None), None);
        state.locations = Locations::Loading;

        assert_eq!(connect_button(&state), disabled("Connect", false));
    }

    #[test]
    fn busy_states_disable_the_button_with_a_spinner() {
        for selected in [None, Some("de1"), Some("au3")] {
            let connecting = state(
                Status::Connecting {
                    spoke_id: "de1".into(),
                },
                selected,
            );
            let disconnecting = state(
                Status::Disconnecting {
                    spoke_id: "de1".into(),
                },
                selected,
            );

            assert_eq!(connect_button(&connecting), disabled("Connecting…", true));
            assert_eq!(
                connect_button(&disconnecting),
                disabled("Disconnecting…", true)
            );
        }
    }

    #[test]
    fn connected_disconnects_unless_another_location_is_selected() {
        assert_eq!(
            connect_button(&state(connected("de1"), None)),
            enabled("Disconnect", ConnectAction::Disconnect)
        );
        assert_eq!(
            connect_button(&state(connected("de1"), Some("de1"))),
            enabled("Disconnect", ConnectAction::Disconnect)
        );
        assert_eq!(
            connect_button(&state(connected("de1"), Some("au3"))),
            enabled("Switch to Australia - Perth", connect("au3"))
        );
    }

    #[test]
    fn switching_names_an_unknown_location_by_its_spoke_id() {
        let mut state = state(connected("de1"), Some("au3"));
        state.locations = Locations::Failed("offline".into());

        assert_eq!(
            connect_button(&state),
            enabled("Switch to au3", connect("au3"))
        );
    }

    #[test]
    fn nothing_connects_without_a_logged_in_running_helper() {
        let mut logged_out = state(disconnected(Some("de1"), None), None);
        logged_out.account = Some(Account::LoggedOut);
        let mut unknown_account = state(connected("de1"), None);
        unknown_account.account = None;
        let mut not_running = state(disconnected(Some("de1"), None), None);
        not_running.helper = HelperState::NotRunning;

        for state in [logged_out, unknown_account, not_running] {
            assert_eq!(connect_button(&state), disabled("Connect", false));
        }
    }

    #[test]
    fn headline_maps_every_status() {
        let text = |tone, title: &str, detail: &str| Headline {
            tone,
            title: title.into(),
            detail: Detail::Text(detail.into()),
        };

        assert_eq!(
            headline(&state(disconnected(Some("de1"), None), None)),
            text(Tone::Idle, "Disconnected", "Your traffic is not protected.")
        );
        assert_eq!(
            headline(&state(
                disconnected(Some("de1"), Some("handshake timed out")),
                None
            )),
            text(Tone::Failed, "Connection failed", "handshake timed out")
        );
        assert_eq!(
            headline(&state(
                Status::Connecting {
                    spoke_id: "au3".into()
                },
                None
            )),
            text(Tone::Busy, "Connecting…", "Australia - Perth")
        );
        assert_eq!(
            headline(&state(
                Status::Disconnecting {
                    spoke_id: "au3".into()
                },
                None
            )),
            text(Tone::Busy, "Disconnecting…", "Australia - Perth")
        );
        assert_eq!(
            headline(&state(connected("de1"), None)),
            Headline {
                tone: Tone::Protected,
                title: "Secured".into(),
                detail: Detail::Connected {
                    spoke_id: "de1".into(),
                    tunnel_ip: [10, 1, 2, 3].into(),
                    connected_at: 1_760_140_800,
                },
            }
        );
    }

    #[test]
    fn headline_explains_a_missing_helper_or_login() {
        let mut logged_out = state(disconnected(None, None), None);
        logged_out.account = Some(Account::LoggedOut);
        let mut not_running = state(disconnected(None, None), None);
        not_running.helper = HelperState::NotRunning;
        let mut start_failed = state(disconnected(None, None), None);
        start_failed.helper = HelperState::StartFailed("access denied".into());

        assert_eq!(
            headline(&logged_out).detail,
            Detail::Text("Log in from the tray menu to connect.".into())
        );
        assert_eq!(
            headline(&not_running).detail,
            Detail::Text("Waiting for the VyprVPN helper.".into())
        );
        assert_eq!(
            headline(&start_failed),
            Headline {
                tone: Tone::Failed,
                title: "Helper not running".into(),
                detail: Detail::Text("access denied".into()),
            }
        );
    }

    #[test]
    fn location_row_follows_the_selection_and_the_locations() {
        let mut state = state(disconnected(None, None), Some("au3"));
        assert_eq!(location_row(&state), LocationRow::Selected(&locations()[1]));

        state.locations = Locations::Loading;
        assert_eq!(location_row(&state), LocationRow::Loading);

        state.locations = Locations::Failed("offline".into());
        assert_eq!(location_row(&state), LocationRow::Failed("offline"));

        state.locations = Locations::Loaded(Vec::new());
        assert_eq!(location_row(&state), LocationRow::Empty);
    }

    #[test]
    fn formats_the_connected_time() {
        let at = |seconds: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 + seconds);

        assert_eq!(connected_for(1_000, at(0)), "0:00");
        assert_eq!(connected_for(1_000, at(65)), "1:05");
        assert_eq!(connected_for(1_000, at(3_600 + 62)), "1:01:02");
        assert_eq!(connected_for(1_000 + 30, at(0)), "0:00");
    }
}
