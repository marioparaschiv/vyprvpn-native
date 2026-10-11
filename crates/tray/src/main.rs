#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(any(windows, target_os = "macos")))]
compile_error!("the VyprLite tray app supports only Windows and macOS");

mod flags;
mod helper_link;
mod icons;
mod login_window;
mod popover;
#[cfg_attr(target_os = "macos", path = "macos/service.rs")]
mod service;
mod theme;
mod tray_menu;
mod trust;

use std::collections::BTreeSet;
#[cfg(target_os = "macos")]
use std::ffi::OsString;
#[cfg(windows)]
use std::io;
use std::process::ExitCode;
use std::time::Instant;

use gpui_kit::{
    AnyWindowHandle, App, AsyncApp, Bounds, Entity, GraphicalEnvironment, QuitMode,
    WindowingRequest,
};
use ipc::{Account, Location, Method};
use placement::PhysicalRect;
use tokio::sync::{mpsc, watch};
use tray_icon::menu::{MenuEvent, MenuId};
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

use helper_link::{Helper, LinkState, Locations, RequestError, Requests};
use login_window::LoginWindow;
use popover::toggle::{self, Toggle};
use popover::{Popover, PopoverEvent, anchor};
use tray_menu::{MenuAction, TrayError, TrayMenu};

#[derive(Debug, thiserror::Error)]
enum ArgsError {
    #[error("usage: VyprLite [--console]")]
    Usage,
    #[cfg(windows)]
    #[error("finding the console pipe failed: {0}")]
    ConsolePipe(#[from] io::Error),
    #[cfg(target_os = "macos")]
    #[error("the console socket path {0:?} is not valid UTF-8")]
    ConsoleSocketPath(OsString),
}

enum UiEvent {
    MenuClicked(MenuId),
    /// A left click on the tray icon, which is at this rectangle.
    TrayClicked(PhysicalRect),
    Popover(PopoverEvent),
    WindowClosed,
}

/// The popover while it is open, and when it last closed.
#[derive(Default)]
struct PopoverSlot {
    open: Option<(AnyWindowHandle, Entity<Popover>)>,
    closed_at: Option<Instant>,
}

impl PopoverSlot {
    fn window(&self) -> Option<AnyWindowHandle> {
        self.open.as_ref().map(|(window, _)| *window)
    }

    /// Notices the popover having closed itself, on Escape or losing focus.
    fn notice_closed(&mut self, cx: &mut AsyncApp) {
        let Some(window) = self.window() else {
            return;
        };
        if cx.update(|cx| cx.windows().contains(&window)) {
            return;
        }

        self.open = None;
        self.closed_at = Some(cx.background_executor().now());
    }
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    #[cfg(debug_assertions)]
    if let Ok(page) = std::env::var("VYPRLITE_DEV_POPOVER") {
        return popover::dev::run(&page);
    }

    let helper = match helper(std::env::args().skip(1)) {
        Ok(helper) => helper,
        Err(error) => {
            log::error!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let (observed, requests) = match helper_link::spawn(helper) {
        Ok(link) => link,
        Err(error) => {
            log::error!("starting the helper link failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    // Headless skips DirectX and its per-vsync wakeups; the app goes windowed only while the login
    // window is open.
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .with_windowing(WindowingRequest::Headless)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            popover::init(cx);

            if let Err(error) = start(cx, observed, requests) {
                log::error!("{error}");
                cx.quit();
            }
        });

    ExitCode::SUCCESS
}

/// `--console` talks to a helper started with `vyprlite-helper --console` instead of the service.
fn helper(mut args: impl Iterator<Item = String>) -> Result<Helper, ArgsError> {
    match (args.next().as_deref(), args.next()) {
        (None, _) => Ok(Helper::Service),
        #[cfg(windows)]
        (Some("--console"), None) => {
            let user_sid = ipc::current_user_sid()?;
            Ok(Helper::Console {
                pipe_name: ipc::console_pipe_name(&user_sid),
                user_sid,
            })
        }
        #[cfg(target_os = "macos")]
        (Some("--console"), None) => Ok(Helper::Console {
            pipe_name: ipc::console_socket_path()
                .into_os_string()
                .into_string()
                .map_err(ArgsError::ConsoleSocketPath)?,
        }),
        _ => Err(ArgsError::Usage),
    }
}

fn start(
    cx: &mut App,
    mut observed: watch::Receiver<LinkState>,
    requests: Requests,
) -> Result<(), TrayError> {
    let mut tray = TrayMenu::new()?;
    if let Err(error) = theme::follow_system(cx) {
        log::warn!("{error}");
    }

    let (events, mut received) = mpsc::unbounded_channel();
    MenuEvent::set_event_handler(Some({
        let events = events.clone();
        move |event: MenuEvent| {
            // Fails only once the UI loop below has ended and the app is quitting.
            events.send(UiEvent::MenuClicked(event.id)).ok();
        }
    }));
    TrayIconEvent::set_event_handler(Some({
        let events = events.clone();
        move |event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                rect,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                events
                    .send(UiEvent::TrayClicked(anchor::icon_rect(rect)))
                    .ok();
            }
        }
    }));
    cx.on_window_closed({
        let events = events.clone();
        move |_, _| {
            events.send(UiEvent::WindowClosed).ok();
        }
    })
    .detach();

    cx.spawn(async move |cx: &mut AsyncApp| {
        let mut asked_to_log_in = false;
        let mut flags_cached = false;
        let mut popover = PopoverSlot::default();

        loop {
            tokio::select! {
                changed = observed.changed() => {
                    if changed.is_err() {
                        log::error!("the helper link stopped");
                        break;
                    }

                    let state = observed.borrow_and_update().clone();
                    log::info!("helper: {:?}, account: {:?}", state.helper, state.account);
                    if let Err(error) = tray.show(&state) {
                        log::error!("{error}");
                    }
                    if let Some((window, view)) = &popover.open {
                        show_in_popover(cx, *window, view, state.clone());
                    }

                    if !flags_cached && let Locations::Loaded(locations) = &state.locations {
                        flags_cached = true;
                        if let Err(error) = flags::cache(country_codes(locations)) {
                            log::warn!("cannot cache flags: {error}");
                        }
                    }

                    if !asked_to_log_in && state.account == Some(Account::LoggedOut) {
                        asked_to_log_in = true;
                        show_login(cx, &requests, popover.window()).await;
                    }
                }
                Some(event) = received.recv() => match event {
                    UiEvent::MenuClicked(id) => match tray.action(&id) {
                        Some(MenuAction::Connect(spoke_id)) => connect(cx, &requests, spoke_id),
                        Some(MenuAction::Disconnect) => disconnect(cx, &requests),
                        Some(MenuAction::LogIn) => {
                            show_login(cx, &requests, popover.window()).await;
                        }
                        Some(MenuAction::LogOut) => log_out(cx, &requests),
                        Some(MenuAction::Exit) => break,
                        None => {}
                    },
                    UiEvent::TrayClicked(icon) => {
                        popover.notice_closed(cx);
                        let now = cx.background_executor().now();

                        match toggle::on_click(popover.open.is_some(), popover.closed_at, now) {
                            Toggle::Open => {
                                let state = observed.borrow().clone();
                                popover.open = open_popover(cx, icon, state, &events).await;
                            }
                            Toggle::Close => {
                                if let Some(window) = popover.window() {
                                    close_popover(cx, window);
                                }
                            }
                            Toggle::Nothing => {}
                        }
                    }
                    UiEvent::Popover(PopoverEvent::Select(spoke_id)) => requests.select(spoke_id),
                    UiEvent::Popover(PopoverEvent::Connect(spoke_id)) => {
                        connect(cx, &requests, spoke_id);
                    }
                    UiEvent::Popover(PopoverEvent::Disconnect) => disconnect(cx, &requests),
                    UiEvent::WindowClosed => {
                        popover.notice_closed(cx);
                        go_headless_once_windowless(cx).await;
                    }
                },
            }
        }

        cx.update(|cx| cx.quit());
    })
    .detach();

    Ok(())
}

fn country_codes(locations: &[Location]) -> Vec<String> {
    let codes: BTreeSet<&str> = locations
        .iter()
        .map(|location| location.country_code.as_str())
        .collect();

    codes.into_iter().map(str::to_owned).collect()
}

/// Brings the login window forward, opening it if needed.
async fn show_login(cx: &mut AsyncApp, requests: &Requests, popover: Option<AnyWindowHandle>) {
    let open: Option<AnyWindowHandle> = cx.update(|cx| {
        cx.windows()
            .into_iter()
            .find(|window| Some(*window) != popover)
    });
    if let Some(window) = open {
        if let Err(error) = window.update(cx, |_, window, _| window.activate_window()) {
            log::warn!("bringing the login window forward failed: {error}");
        }
        return;
    }

    if !go_windowed(cx).await {
        return;
    }
    if let Err(error) = cx.update(|cx| LoginWindow::open(cx, requests.clone())) {
        log::error!("opening the login window failed: {error:#}");
        go_headless_once_windowless(cx).await;
    }
}

/// Opens the popover against the tray icon at `icon`, forwarding what it asks for as
/// [`UiEvent::Popover`].
async fn open_popover(
    cx: &mut AsyncApp,
    icon: PhysicalRect,
    state: LinkState,
    events: &mpsc::UnboundedSender<UiEvent>,
) -> Option<(AnyWindowHandle, Entity<Popover>)> {
    if !go_windowed(cx).await {
        return None;
    }

    let events = events.clone();
    let opened = cx.update(|cx| -> gpui_kit::Result<_> {
        let (display, origin) = match anchor::monitor(icon, cx) {
            Ok(monitor) => {
                let anchor = anchor::anchor(icon, &monitor);
                (anchor.display, anchor.origin)
            }
            Err(error) => {
                log::warn!("{error}; centring the popover");
                let centred = Bounds::centered(None, Popover::window_size(), cx);
                (None, centred.origin)
            }
        };
        let (window, popover) = Popover::open(cx, display, origin, state)?;
        cx.subscribe(&popover, move |_, event: &PopoverEvent, _| {
            // Fails only once the UI loop has ended and the app is quitting.
            events.send(UiEvent::Popover(event.clone())).ok();
        })
        .detach();

        Ok((window, popover))
    });

    match opened {
        Ok(opened) => Some(opened),
        Err(error) => {
            log::error!("opening the popover failed: {error:#}");
            go_headless_once_windowless(cx).await;
            None
        }
    }
}

fn show_in_popover(
    cx: &mut AsyncApp,
    window: AnyWindowHandle,
    popover: &Entity<Popover>,
    state: LinkState,
) {
    let shown = window.update(cx, |_, window, cx| {
        popover.update(cx, |popover, cx| popover.set_state(state, window, cx));
    });
    // Fails only when the popover has just closed itself; its WindowClosed event follows.
    if let Err(error) = shown {
        log::debug!("the popover closed before showing the new state: {error}");
    }
}

fn close_popover(cx: &mut AsyncApp, window: AnyWindowHandle) {
    if let Err(error) = window.update(cx, |_, window, _| window.remove_window()) {
        log::debug!("the popover had already closed: {error}");
    }
}

/// Returns whether windows can be shown.
async fn go_windowed(cx: &mut AsyncApp) -> bool {
    let windowed = cx.update(|cx| {
        cx.request_windowing(WindowingRequest::Windowed(GraphicalEnvironment::detect()))
    });
    if let Err(error) = windowed.await {
        log::error!("cannot show windows: {error:#}");
        return false;
    }
    // Going windowed does not activate a macOS app, so the window would open behind the others.
    #[cfg(target_os = "macos")]
    cx.update(|cx| cx.activate(true));

    true
}

fn log_out(cx: &mut AsyncApp, requests: &Requests) {
    let requests = requests.clone();

    cx.spawn(async move |_| {
        if let Err(error) = requests.send(Method::Logout).await {
            log::error!("logging out failed: {error}");
        }
    })
    .detach();
}

fn connect(cx: &mut AsyncApp, requests: &Requests, spoke_id: String) {
    let requests = requests.clone();
    carry_out(cx, async move { requests.connect(spoke_id).await });
}

fn disconnect(cx: &mut AsyncApp, requests: &Requests) {
    let requests = requests.clone();
    carry_out(cx, async move { requests.disconnect().await });
}

/// Runs a connect or disconnect request, showing why if it fails; its progress shows in the status.
fn carry_out(cx: &mut AsyncApp, request: impl Future<Output = Result<(), RequestError>> + 'static) {
    cx.spawn(async move |_| {
        if let Err(error) = request.await {
            log::error!("{error}");
            tray_menu::notify(&error.to_string());
        }
    })
    .detach();
}

async fn go_headless_once_windowless(cx: &mut AsyncApp) {
    if !cx.update(|cx| cx.windows().is_empty()) {
        return;
    }

    let headless = cx.update(|cx| cx.request_windowing(WindowingRequest::Headless));
    if let Err(error) = headless.await {
        log::error!("going headless failed: {error:#}");
    }
}
