//! `VYPRLITE_DEV_POPOVER=disconnected|connecting|connected|error|subpage` opens the popover on
//! made-up state in debug builds, for screenshots and trying it out without a helper.

use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use gpui_kit::*;
use ipc::{Account, Reply, Status};

use super::{Popover, PopoverEvent};
use crate::helper_link::{HelperState, LinkState, Locations};
use crate::theme;

const LOCATIONS: &str = include_str!("../../../vypr-api/tests/fixtures/locations.json");

#[derive(Debug, thiserror::Error)]
pub enum DevError {
    #[error(
        "VYPRLITE_DEV_POPOVER={0:?}; use disconnected, connecting, connected, error or subpage"
    )]
    Page(String),
    #[error("the bundled locations fixture is malformed: {0}")]
    Fixture(#[from] ipc::CodecError),
    #[error("the bundled locations fixture is not a locations reply")]
    NotLocations,
}

pub fn run(page: &str) -> ExitCode {
    let state = match state(page) {
        Ok(state) => state,
        Err(error) => {
            log::error!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let subpage = page == "subpage";

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            super::init(cx);
            if let Err(error) = theme::follow_system(cx) {
                log::warn!("{error}");
            }
            cx.on_window_closed(|cx, _| cx.quit()).detach();

            if let Err(error) = open(cx, state, subpage) {
                log::error!("opening the popover failed: {error:#}");
                cx.quit();
            }
        });

    ExitCode::SUCCESS
}

fn open(cx: &mut App, state: LinkState, subpage: bool) -> Result<()> {
    let origin = Bounds::centered(None, Popover::window_size(), cx).origin;
    let (window, popover) = Popover::open(cx, None, origin, state)?;

    if subpage {
        window.update(cx, |_, window, cx| {
            popover.update(cx, |popover, cx| popover.show_locations(window, cx));
        })?;
    }

    cx.subscribe(&popover, move |popover, event: &PopoverEvent, cx| {
        log::info!("popover asked: {event:?}");
        let mut state = popover.read(cx).state.clone();
        match event {
            PopoverEvent::Select(spoke_id) => state.selected = Some(spoke_id.clone()),
            PopoverEvent::Connect(spoke_id) => {
                state.helper = HelperState::Running(Status::Connecting {
                    spoke_id: spoke_id.clone(),
                });
            }
            PopoverEvent::Disconnect => {
                state.helper = HelperState::Running(Status::Disconnected {
                    last_spoke_id: state.selected_spoke_id().map(str::to_owned),
                    error: None,
                });
            }
        }

        window
            .update(cx, |_, window, cx| {
                popover.update(cx, |popover, cx| popover.set_state(state, window, cx));
            })
            .ok();
    })
    .detach();

    Ok(())
}

fn state(page: &str) -> Result<LinkState, DevError> {
    let Reply::Locations(locations) = ipc::decode(LOCATIONS)? else {
        return Err(DevError::NotLocations);
    };
    let spoke_id = || "de1".to_owned();

    let status = match page {
        "disconnected" | "subpage" => Status::Disconnected {
            last_spoke_id: Some(spoke_id()),
            error: None,
        },
        "connecting" => Status::Connecting {
            spoke_id: spoke_id(),
        },
        "connected" => Status::Connected {
            spoke_id: spoke_id(),
            tunnel_ip: [10, 1, 2, 3].into(),
            connected_at: (SystemTime::now() - Duration::from_secs(754))
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        },
        "error" => Status::Disconnected {
            last_spoke_id: Some(spoke_id()),
            error: Some("The VPN server did not answer the handshake.".to_owned()),
        },
        other => return Err(DevError::Page(other.to_owned())),
    };

    Ok(LinkState {
        helper: HelperState::Running(status),
        account: Some(Account::LoggedIn {
            username: "dev".to_owned(),
        }),
        locations: Locations::Loaded(locations),
        selected: None,
    })
}
