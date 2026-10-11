//! `VYPRLITE_DEV_POPOVER=disconnected|connecting|connected|error|subpage` opens the popover on
//! made-up state in debug builds, for screenshots and trying it out without a helper.

use std::cell::Cell;
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui_kit::*;
use ipc::{Account, Status};

use super::anchor::Anchor;
use super::{Popover, PopoverEvent, recorded_locations};
use crate::helper_link::{HelperState, LinkState, Locations};
use crate::theme;

#[derive(Debug, thiserror::Error)]
pub enum DevError {
    #[error(
        "VYPRLITE_DEV_POPOVER={0:?}; use disconnected, connecting, connected, error or subpage"
    )]
    Page(String),
    #[error("the bundled locations fixture is malformed: {0}")]
    Fixture(#[from] ipc::CodecError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preview {
    Disconnected,
    Connecting,
    Connected,
    Error,
    Subpage,
}

impl Preview {
    fn parse(page: &str) -> Result<Self, DevError> {
        match page {
            "disconnected" => Ok(Self::Disconnected),
            "connecting" => Ok(Self::Connecting),
            "connected" => Ok(Self::Connected),
            "error" => Ok(Self::Error),
            "subpage" => Ok(Self::Subpage),
            other => Err(DevError::Page(other.to_owned())),
        }
    }
}

pub fn run(page: &str) -> ExitCode {
    let prepared = Preview::parse(page).and_then(|preview| Ok((preview, state(preview)?)));
    let (preview, state) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            log::error!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let failed = Rc::new(Cell::new(false));

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run({
            let failed = failed.clone();
            move |cx| {
                gpui_kit::init(cx);
                super::init(cx);
                if let Err(error) = theme::follow_system(cx) {
                    log::warn!("{error}");
                }
                cx.on_window_closed(|cx, _| cx.quit()).detach();

                if let Err(error) = open(cx, state, preview == Preview::Subpage) {
                    log::error!("opening the popover failed: {error:#}");
                    failed.set(true);
                    cx.quit();
                }
            }
        });

    if failed.get() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn open(cx: &mut App, state: LinkState, subpage: bool) -> Result<()> {
    let (window, popover) = Popover::open(cx, Anchor::centred(cx), state)?;

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

        let shown = window.update(cx, |_, _, cx| {
            popover.update(cx, |popover, cx| popover.set_state(state, cx));
        });
        if let Err(error) = shown {
            log::debug!("the popover closed before showing the new state: {error}");
        }
    })
    .detach();

    Ok(())
}

fn state(preview: Preview) -> Result<LinkState, DevError> {
    let locations = recorded_locations()?;
    let spoke_id = || "de1".to_owned();

    let status = match preview {
        Preview::Disconnected | Preview::Subpage => Status::Disconnected {
            last_spoke_id: Some(spoke_id()),
            error: None,
        },
        Preview::Connecting => Status::Connecting {
            spoke_id: spoke_id(),
        },
        Preview::Connected => Status::Connected {
            spoke_id: spoke_id(),
            tunnel_ip: [10, 1, 2, 3].into(),
            connected_at: (SystemTime::now() - Duration::from_secs(754))
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        },
        Preview::Error => Status::Disconnected {
            last_spoke_id: Some(spoke_id()),
            error: Some("The VPN server did not answer the handshake.".to_owned()),
        },
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
