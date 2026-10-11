use ipc::{Account, Status};
use notify_rust::Notification;
use tray_icon::menu::{self, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{BadIcon, TrayIcon, TrayIconBuilder};

use crate::helper_link::{HelperState, LinkState, Locations};
use crate::icons::{self, IconKind};

#[derive(Debug, thiserror::Error)]
pub enum TrayError {
    #[error("building the tray icon image failed: {0}")]
    Icon(#[from] BadIcon),
    #[error("building the tray menu failed: {0}")]
    Menu(#[from] menu::Error),
    #[error("updating the tray icon failed: {0}")]
    TrayIcon(#[from] tray_icon::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    Connect(String),
    Disconnect,
    LogIn,
    LogOut,
    Exit,
}

/// The notification-area icon and its right-click menu. Must live on the UI thread.
///
/// Items are updated in place rather than rebuilt, so an open menu stays intact.
pub struct TrayMenu {
    icon: TrayIcon,
    status: MenuItem,
    connect: MenuItem,
    disconnect: MenuItem,
    account: MenuItem,
    exit: MenuId,
    shown: MenuView,
    errors: ErrorNotices,
}

impl TrayMenu {
    pub fn new() -> Result<Self, TrayError> {
        let shown = MenuView::new(&LinkState::default());
        let status = MenuItem::new(&shown.status, false, None);
        let connect = MenuItem::new(&shown.connect_label, shown.connect_to.is_some(), None);
        let disconnect = MenuItem::new("Disconnect", shown.disconnect_enabled, None);
        let account = MenuItem::new(&shown.account_label, shown.account.is_some(), None);
        let exit = MenuItem::new("Exit", true, None);
        let menu = Menu::with_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &connect,
            &disconnect,
            &PredefinedMenuItem::separator(),
            &account,
            &exit,
        ])?;
        let icon = TrayIconBuilder::new()
            .with_icon(icons::icon(shown.icon)?)
            .with_tooltip(&shown.tooltip)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()?;

        Ok(Self {
            icon,
            status,
            connect,
            disconnect,
            account,
            exit: exit.into_id(),
            shown,
            errors: ErrorNotices::default(),
        })
    }

    pub fn action(&self, id: &MenuId) -> Option<MenuAction> {
        if *id == self.exit {
            Some(MenuAction::Exit)
        } else if id == self.connect.id() {
            self.shown.connect_to.clone().map(MenuAction::Connect)
        } else if id == self.disconnect.id() {
            self.shown
                .disconnect_enabled
                .then_some(MenuAction::Disconnect)
        } else if id == self.account.id() {
            self.shown.account.clone()
        } else {
            None
        }
    }

    pub fn show(&mut self, state: &LinkState) -> Result<(), TrayError> {
        if let Some(error) = self.errors.take(&state.helper) {
            notify(&error);
        }

        let view = MenuView::new(state);
        self.status.set_text(&view.status);
        self.connect.set_text(&view.connect_label);
        self.connect.set_enabled(view.connect_to.is_some());
        self.disconnect.set_enabled(view.disconnect_enabled);
        self.account.set_text(&view.account_label);
        self.account.set_enabled(view.account.is_some());
        if view.tooltip != self.shown.tooltip {
            self.icon.set_tooltip(Some(&view.tooltip))?;
        }
        if view.icon != self.shown.icon {
            self.icon.set_icon(Some(icons::icon(view.icon)?))?;
        }
        self.shown = view;

        Ok(())
    }
}

/// Shows `message` as a desktop notification from VyprLite.
pub fn notify(message: &str) {
    let mut notification = Notification::new();
    notification.summary("VyprLite").body(message);
    #[cfg(windows)]
    notification.app_id(ipc::APP_USER_MODEL_ID);

    if let Err(error) = notification.show() {
        log::warn!("showing the notification {message:?} failed: {error}");
    }
}

/// What the tray shows for a [`LinkState`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct MenuView {
    status: String,
    tooltip: String,
    icon: IconKind,
    connect_label: String,
    /// Where Connect goes; `None` disables it.
    connect_to: Option<String>,
    disconnect_enabled: bool,
    account_label: String,
    /// `None` disables the item.
    account: Option<MenuAction>,
}

impl MenuView {
    fn new(state: &LinkState) -> Self {
        let status = match &state.helper {
            HelperState::Running(status) => Some(status),
            HelperState::NotRunning | HelperState::StartFailed(_) | HelperState::Untrusted(_) => {
                None
            }
        };
        let logged_in = matches!(state.account, Some(Account::LoggedIn { .. }));
        let connected_to = match status {
            Some(Status::Connected { spoke_id, .. }) => Some(spoke_id.as_str()),
            _ => None,
        };
        let idle = matches!(
            status,
            Some(Status::Disconnected { .. } | Status::Connected { .. })
        );
        let target = state.selected_spoke_id();
        let status_label = status_label(&state.helper, &state.locations);

        Self {
            tooltip: format!("VyprLite\n{status_label}"),
            status: status_label,
            icon: match status {
                Some(Status::Connected { .. }) => IconKind::Connected,
                Some(Status::Connecting { .. } | Status::Disconnecting { .. }) => IconKind::Busy,
                Some(Status::Disconnected { .. }) | None => IconKind::Disconnected,
            },
            connect_label: match target {
                Some(spoke_id) => {
                    format!("Connect to {}", location_name(&state.locations, spoke_id))
                }
                None => "Connect".to_owned(),
            },
            connect_to: target
                .filter(|&target| logged_in && idle && connected_to != Some(target))
                .map(str::to_owned),
            disconnect_enabled: logged_in && connected_to.is_some(),
            account_label: match &state.account {
                Some(Account::LoggedIn { username }) => format!("Log out ({username})"),
                Some(Account::LoggedOut) | None => "Log in…".to_owned(),
            },
            account: state.account.as_ref().map(|account| match account {
                Account::LoggedIn { .. } => MenuAction::LogOut,
                Account::LoggedOut => MenuAction::LogIn,
            }),
        }
    }
}

fn status_label(state: &HelperState, locations: &Locations) -> String {
    match state {
        HelperState::NotRunning => "Helper not running".to_owned(),
        HelperState::StartFailed(reason) => format!("Helper not running: {reason}"),
        HelperState::Untrusted(untrusted) => format!("Helper not trusted: {untrusted}"),
        HelperState::Running(Status::Disconnected { .. }) => "Disconnected".to_owned(),
        HelperState::Running(Status::Connecting { spoke_id }) => {
            format!("Connecting to {}…", location_name(locations, spoke_id))
        }
        HelperState::Running(Status::Connected { spoke_id, .. }) => {
            format!("Connected: {}", location_name(locations, spoke_id))
        }
        HelperState::Running(Status::Disconnecting { .. }) => "Disconnecting…".to_owned(),
    }
}

/// The location's name, or its spoke id while the locations are unknown.
fn location_name<'a>(locations: &'a Locations, spoke_id: &'a str) -> &'a str {
    match locations {
        Locations::Loaded(locations) => locations
            .iter()
            .find(|location| location.spoke_id == spoke_id)
            .map_or(spoke_id, |location| &location.name),
        Locations::Loading | Locations::Failed(_) => spoke_id,
    }
}

/// Picks out each connection error the helper reports, once.
#[derive(Debug, Default)]
struct ErrorNotices {
    shown: Option<String>,
}

impl ErrorNotices {
    /// The error to notify about, if `state` reports one not notified yet.
    ///
    /// The helper keeps reporting an error until the next connect, so it is forgotten only once
    /// the helper reports anything else; not while the helper is unreachable, since it reports the
    /// same error again after a restart of either side.
    fn take(&mut self, state: &HelperState) -> Option<String> {
        let HelperState::Running(status) = state else {
            return None;
        };
        let error = match status {
            Status::Disconnected { error, .. } => error.clone(),
            Status::Connecting { .. } | Status::Connected { .. } | Status::Disconnecting { .. } => {
                None
            }
        };
        if error == self.shown {
            return None;
        }

        self.shown.clone_from(&error);
        error
    }
}

#[cfg(test)]
mod tests {
    use ipc::Location;

    use super::*;
    use crate::helper_link::Requests;

    fn germany() -> Location {
        Location {
            name: "Germany".into(),
            country: "Germany".into(),
            country_code: "GER".into(),
            region: "Europe".into(),
            spoke_id: "de1".into(),
            hostname: "de1.vpn.goldenfrog.com".into(),
        }
    }

    fn perth() -> Location {
        Location {
            name: "Australia - Perth".into(),
            country: "Australia".into(),
            country_code: "AUS".into(),
            region: "Oceania".into(),
            spoke_id: "au3".into(),
            hostname: "au3.vpn.goldenfrog.com".into(),
        }
    }

    fn alice() -> Option<Account> {
        Some(Account::LoggedIn {
            username: "alice".into(),
        })
    }

    fn running(status: Status) -> LinkState {
        LinkState {
            helper: HelperState::Running(status),
            account: alice(),
            locations: Locations::Loaded(vec![germany(), perth()]),
            selected: None,
        }
    }

    fn disconnected(error: Option<&str>) -> Status {
        Status::Disconnected {
            last_spoke_id: Some("au3".into()),
            error: error.map(str::to_owned),
        }
    }

    fn connecting() -> Status {
        Status::Connecting {
            spoke_id: "de1".into(),
        }
    }

    fn connected() -> Status {
        Status::Connected {
            spoke_id: "de1".into(),
            tunnel_ip: [10, 1, 2, 3].into(),
            connected_at: 1_760_140_800,
        }
    }

    fn disconnecting() -> Status {
        Status::Disconnecting {
            spoke_id: "de1".into(),
        }
    }

    #[test]
    fn disconnected_offers_the_last_location() {
        assert_eq!(
            MenuView::new(&running(disconnected(None))),
            MenuView {
                status: "Disconnected".into(),
                tooltip: "VyprLite\nDisconnected".into(),
                icon: IconKind::Disconnected,
                connect_label: "Connect to Australia - Perth".into(),
                connect_to: Some("au3".into()),
                disconnect_enabled: false,
                account_label: "Log out (alice)".into(),
                account: Some(MenuAction::LogOut),
            }
        );
    }

    #[test]
    fn connecting_is_busy() {
        let view = MenuView::new(&running(connecting()));

        assert_eq!(view.status, "Connecting to Germany…");
        assert_eq!(view.tooltip, "VyprLite\nConnecting to Germany…");
        assert_eq!(view.icon, IconKind::Busy);
        assert_eq!(view.connect_label, "Connect to Germany");
        assert_eq!(view.connect_to, None);
        assert!(!view.disconnect_enabled);
    }

    #[test]
    fn connected_offers_disconnect() {
        let view = MenuView::new(&running(connected()));

        assert_eq!(view.status, "Connected: Germany");
        assert_eq!(view.tooltip, "VyprLite\nConnected: Germany");
        assert_eq!(view.icon, IconKind::Connected);
        assert_eq!(view.connect_label, "Connect to Germany");
        assert_eq!(view.connect_to, None);
        assert!(view.disconnect_enabled);
    }

    #[test]
    fn connected_offers_switching_to_another_selected_location() {
        let view = MenuView::new(&LinkState {
            selected: Some("au3".into()),
            ..running(connected())
        });

        assert_eq!(view.connect_label, "Connect to Australia - Perth");
        assert_eq!(view.connect_to, Some("au3".into()));
        assert!(view.disconnect_enabled);
    }

    #[test]
    fn a_location_picked_in_the_popover_relabels_connect() {
        let (requests, _commands) = Requests::intercepted();
        let observed = requests.observe();

        requests.select("de1".into());
        let view = MenuView::new(&LinkState {
            selected: observed.borrow().selected.clone(),
            ..running(disconnected(None))
        });

        assert_eq!(view.connect_label, "Connect to Germany");
        assert_eq!(view.connect_to, Some("de1".into()));
    }

    #[test]
    fn disconnecting_is_busy() {
        let view = MenuView::new(&running(disconnecting()));

        assert_eq!(view.status, "Disconnecting…");
        assert_eq!(view.icon, IconKind::Busy);
        assert_eq!(view.connect_to, None);
        assert!(!view.disconnect_enabled);
    }

    #[test]
    fn logged_out_disables_connect_and_disconnect() {
        for status in [disconnected(None), connected()] {
            let view = MenuView::new(&LinkState {
                account: Some(Account::LoggedOut),
                ..running(status)
            });

            assert_eq!(view.connect_to, None);
            assert!(!view.disconnect_enabled);
            assert_eq!(view.account_label, "Log in…");
            assert_eq!(view.account, Some(MenuAction::LogIn));
        }
    }

    #[test]
    fn shows_the_spoke_id_of_an_unknown_location() {
        let view = MenuView::new(&LinkState {
            locations: Locations::Loading,
            ..running(connected())
        });

        assert_eq!(view.status, "Connected: de1");
        assert_eq!(view.connect_label, "Connect to de1");

        let view = MenuView::new(&LinkState {
            locations: Locations::Loaded(vec![perth()]),
            ..running(connecting())
        });

        assert_eq!(view.status, "Connecting to de1…");
    }

    #[test]
    fn without_a_location_connect_is_disabled() {
        let view = MenuView::new(&LinkState {
            locations: Locations::Failed("offline".into()),
            ..running(Status::Disconnected {
                last_spoke_id: None,
                error: None,
            })
        });

        assert_eq!(view.connect_label, "Connect");
        assert_eq!(view.connect_to, None);
    }

    #[test]
    fn helper_not_running_disables_everything_but_exit() {
        assert_eq!(
            MenuView::new(&LinkState::default()),
            MenuView {
                status: "Helper not running".into(),
                tooltip: "VyprLite\nHelper not running".into(),
                icon: IconKind::Disconnected,
                connect_label: "Connect".into(),
                connect_to: None,
                disconnect_enabled: false,
                account_label: "Log in…".into(),
                account: None,
            }
        );
    }

    #[test]
    fn notifies_each_error_once() {
        let mut errors = ErrorNotices::default();
        let failed = HelperState::Running(disconnected(Some("handshake timed out")));

        assert_eq!(errors.take(&HelperState::Running(disconnected(None))), None);
        assert_eq!(errors.take(&failed), Some("handshake timed out".into()));
        assert_eq!(errors.take(&failed), None);

        assert_eq!(errors.take(&HelperState::NotRunning), None);
        assert_eq!(errors.take(&failed), None);

        assert_eq!(errors.take(&HelperState::Running(connecting())), None);
        assert_eq!(errors.take(&failed), Some("handshake timed out".into()));
    }
}
