//! The popover the tray icon's left click opens: connection status, the location to connect to,
//! and the Connect button, with a subpage for picking the location.

pub mod anchor;
mod backdrop;
#[cfg(debug_assertions)]
pub mod dev;
#[cfg(target_os = "windows")]
mod foreground;
mod globe;
#[cfg(target_os = "windows")]
mod hwnd;
mod location_list;
mod model;
mod presence;
mod smooth_scroll;
pub mod toggle;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::base::actions::Cancel;
use gpui_kit::base::{Transition, transition};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Escape;
use gpui_kit::component::list::{List, ListEvent, ListState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::helper_link::{HelperState, LinkState, Locations};
use crate::{flags, theme};
use anchor::Anchor;
use backdrop::Surface;
use globe::Globe;
use location_list::LocationList;
use model::{ConnectAction, Detail, Headline, LocationRow, Tone};
use presence::{Presence, Side};
use smooth_scroll::SmoothScroll;

const CONTEXT: &str = "LocationPopover";

/// The card's size, which is the window's: the OS draws its shadow.
const CARD: Size<Pixels> = size(px(340.), px(420.));

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Cancel, Some(CONTEXT))]);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopoverEvent {
    /// The user picked where Connect goes.
    Select(String),
    Connect(String),
    Disconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Main,
    Locations,
}

/// A borderless window showing the [`LinkState`] it is handed. It asks for changes through
/// [`PopoverEvent`]s, and closes on Escape or when it loses focus.
pub struct Popover {
    state: LinkState,
    page: Page,
    locations: Entity<ListState<LocationList>>,
    focus_handle: FocusHandle,
    flag_dir: Option<PathBuf>,
    presence: Presence,
    toward_icon: Option<Side>,
    surface: Surface,
    globe: Globe,
    /// Redraws the connected-for time each second while connected.
    _ticks: Option<Task<()>>,
    /// Removes the window once the exit has played.
    _closing: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PopoverEvent> for Popover {}

impl Popover {
    pub fn open(
        cx: &mut App,
        anchor: Anchor,
        state: LinkState,
    ) -> Result<(AnyWindowHandle, Entity<Self>)> {
        let surface = Surface::current();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: anchor.origin,
                size: CARD,
            })),
            display_id: anchor.display,
            titlebar: None,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            window_background: surface.window_background(),
            ..Default::default()
        };

        let flag_dir = flags::cache_dir()
            .inspect_err(|error| log::warn!("flags are unavailable: {error}"))
            .ok();
        let (window, popover) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Self {
                toward_icon: anchor.toward_icon,
                surface,
                ..Self::new(state, flag_dir, window, cx)
            })
        })?;
        // `cx.activate` does nothing on Windows; there the window takes the foreground itself.
        cx.activate(true);
        #[cfg(target_os = "windows")]
        window.update(cx, |_, window, cx| match hwnd::of(window) {
            Ok(hwnd) => {
                if let Err(error) = backdrop::apply(&hwnd, surface) {
                    log::warn!("showing the solid card: {error}");
                    popover.update(cx, |popover, cx| {
                        popover.surface = Surface::Solid;
                        cx.notify();
                    });
                }
                if let Err(error) = foreground::take(&hwnd) {
                    log::warn!("the popover may close on its first click: {error}");
                }
            }
            Err(error) => log::warn!("the popover's window can't be styled or focused: {error}"),
        })?;

        Ok((window, popover))
    }

    fn new(
        state: LinkState,
        flag_dir: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let locations = cx.new(|cx| {
            ListState::new(LocationList::new(flag_dir.clone()), window, cx).searchable(true)
        });
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let _subscriptions = vec![
            cx.subscribe_in(&locations, window, Self::on_list_event),
            cx.observe_window_activation(window, Self::on_activation_change),
            theme::observe_window(window),
        ];

        let now = cx.background_executor().now();
        let mut popover = Self {
            state: LinkState::default(),
            page: Page::Main,
            locations,
            focus_handle,
            flag_dir,
            presence: Presence::Entering { since: now },
            toward_icon: None,
            surface: Surface::default(),
            globe: Globe::new(globe::target(&state), now, cx.reduce_motion()),
            _ticks: None,
            _closing: None,
            _subscriptions,
        };
        popover.set_state(state, cx);

        popover
    }

    pub fn set_state(&mut self, state: LinkState, cx: &mut Context<Self>) {
        if state.locations != self.state.locations {
            let loaded = state.locations.loaded().to_vec();
            self.locations.update(cx, |list, cx| {
                list.delegate_mut().set_locations(loaded);
                cx.notify();
            });
        }

        let connected = matches!(
            state.helper,
            HelperState::Running(ipc::Status::Connected { .. })
        );
        if !connected {
            self._ticks = None;
        } else if self._ticks.is_none() {
            self._ticks = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        }

        self.globe.set_target(
            globe::target(&state),
            cx.background_executor().now(),
            cx.reduce_motion(),
        );
        self.state = state;
        cx.notify();
    }

    fn show_locations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = self.state.selected_spoke_id().map(str::to_owned);

        self.locations.update(cx, |list, cx| {
            list.delegate_mut().set_chosen(chosen.as_deref());
            list.set_query("", window, cx);

            let chosen = chosen.and_then(|spoke_id| list.delegate().index_of(&spoke_id));
            if let Some(index) = chosen {
                list.set_selected_index(Some(index), window, cx);
                list.scroll_to_selected_item(window, cx);
            }
            list.focus(window, cx);
        });

        self.page = Page::Locations;
        cx.notify();
    }

    fn show_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page = Page::Main;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// Windows reports each activation change separately and late, so moving the foreground to
    /// the popover can report it inactive just before reporting it active again. Only an
    /// inactive state that outlasts the changes already reported closes it.
    fn on_activation_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.is_window_active() {
            return;
        }

        cx.spawn_in(window, async |this, cx| {
            this.update_in(cx, |this, window, cx| {
                if !window.is_window_active() {
                    this.close(window, cx);
                }
            })
        })
        .detach();
    }

    fn on_list_event(
        &mut self,
        list: &Entity<ListState<LocationList>>,
        event: &ListEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ListEvent::Confirm(index) = event else {
            return;
        };
        let Some(spoke_id) = list.read(cx).delegate().spoke_id(*index).map(str::to_owned) else {
            return;
        };

        self.state.selected = Some(spoke_id.clone());
        cx.emit(PopoverEvent::Select(spoke_id));
        self.show_main(window, cx);
    }

    fn press_connect(&mut self, cx: &mut Context<Self>) {
        match model::connect_button(&self.state).action {
            Some(ConnectAction::Connect(spoke_id)) => cx.emit(PopoverEvent::Connect(spoke_id)),
            Some(ConnectAction::Disconnect) => cx.emit(PopoverEvent::Disconnect),
            None => {}
        }
    }

    /// Escape steps back from the subpage, else closes the popover.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::Locations => self.show_main(window, cx),
            Page::Main => self.close(window, cx),
        }
    }

    /// Fades the card out, then removes the window.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.presence, Presence::Exiting { .. }) {
            return;
        }

        let motion = cx.theme().motion_tokens();
        let now = cx.background_executor().now();
        let duration = motion.duration_fast;
        let (from, _) = self.presence.shown(now, motion);

        self.presence = Presence::Exiting { since: now, from };
        self._closing = Some(cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(duration).await;
            if let Err(error) = cx.update(|window, _| window.remove_window()) {
                log::debug!("the popover had already closed: {error}");
            }
        }));
        cx.notify();
    }

    fn render_main(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let button = model::connect_button(&self.state);
        let primary = matches!(button.action, Some(ConnectAction::Connect(_)));

        v_flex()
            .size_full()
            .p_4()
            .gap_4()
            .child(self.render_status(&model::headline(&self.state), cx))
            .child(
                v_flex().gap_2().child(self.render_location_row(cx)).child(
                    Button::new("connect")
                        .large()
                        .w_full()
                        .when(primary, ButtonVariants::primary)
                        .label(button.label)
                        .when(button.busy, |this| this.icon(IconName::LoaderCircle))
                        .loading(button.busy)
                        .disabled(button.action.is_none())
                        .on_click(cx.listener(|this, _, _, cx| this.press_connect(cx))),
                ),
            )
    }

    fn render_status(&self, headline: &Headline, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let color = match headline.tone {
            Tone::Idle => theme.muted_foreground,
            Tone::Busy => theme.info,
            Tone::Protected => theme.success,
            Tone::Failed => theme.danger,
        };
        let symbol = match headline.tone {
            Tone::Idle => Icon::new(IconName::Globe).size_8().into_any_element(),
            Tone::Busy => Spinner::new().large().color(color).into_any_element(),
            Tone::Protected => Icon::new(IconName::CircleCheck).size_8().into_any_element(),
            Tone::Failed => Icon::new(IconName::CircleAlert).size_8().into_any_element(),
        };

        v_flex()
            .flex_1()
            .min_h_0()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                h_flex()
                    .size_16()
                    .justify_center()
                    .rounded_full()
                    .bg(color.opacity(0.12))
                    .text_color(color)
                    .child(symbol),
            )
            .child(
                div()
                    .text_lg()
                    .font_semibold()
                    .child(headline.title.clone()),
            )
            .child(match &headline.detail {
                Detail::Text(text) => div()
                    .max_w_full()
                    .text_sm()
                    .text_center()
                    .text_color(theme.muted_foreground)
                    .child(text.clone())
                    .into_any_element(),
                Detail::Connected {
                    spoke_id,
                    tunnel_ip,
                    connected_at,
                } => self
                    .render_connection(spoke_id, *tunnel_ip, *connected_at, cx)
                    .into_any_element(),
            })
    }

    fn render_connection(
        &self,
        spoke_id: &str,
        tunnel_ip: std::net::IpAddr,
        connected_at: u64,
        cx: &App,
    ) -> impl IntoElement {
        let fact = |label: &'static str, value: AnyElement| {
            h_flex()
                .w_full()
                .justify_between()
                .gap_4()
                .child(div().text_color(cx.theme().muted_foreground).child(label))
                .child(value)
        };
        let location = self.state.locations.find(spoke_id);

        v_flex()
            .w_full()
            .px_2()
            .gap_1()
            .text_sm()
            .child(fact(
                "Location",
                h_flex()
                    .gap_2()
                    .min_w_0()
                    .children(
                        location.map(|location| {
                            flag(self.flag_dir.as_deref(), &location.country_code, cx)
                        }),
                    )
                    .child(
                        div().text_ellipsis().child(
                            location
                                .map_or(spoke_id, |location| &location.name)
                                .to_owned(),
                        ),
                    )
                    .into_any_element(),
            ))
            .child(fact("IP address", tunnel_ip.to_string().into_any_element()))
            .child(fact(
                "Connected for",
                div()
                    .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
                    .child(model::connected_for(connected_at, SystemTime::now()))
                    .into_any_element(),
            ))
    }

    fn render_location_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let row = model::location_row(&self.state);
        let placeholder = |text: String| {
            div()
                .flex_1()
                .text_ellipsis()
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        };
        let content = match row {
            LocationRow::Selected(location) => h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(flag(self.flag_dir.as_deref(), &location.country_code, cx))
                .child(div().flex_1().text_ellipsis().child(location.name.clone()))
                .into_any_element(),
            LocationRow::Loading => placeholder("Loading locations…".to_owned()),
            LocationRow::Failed(reason) => placeholder(format!("Locations unavailable: {reason}")),
            LocationRow::Empty => placeholder("Choose a location".to_owned()),
        };

        Button::new("location")
            .outline()
            .large()
            .w_full()
            // The outline fill is see-through in dark mode; over the blur it shows as it does on
            // the solid card.
            .when(self.surface == Surface::Blurred, |button| {
                button.bg(theme.popover.blend(theme.input_background()))
            })
            .disabled(!matches!(self.state.locations, Locations::Loaded(_)))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(content)
                    .child(Icon::new(IconName::ChevronRight).text_color(theme.muted_foreground)),
            )
            .on_click(cx.listener(|this, _, window, cx| this.show_locations(window, cx)))
    }

    fn render_locations(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .gap_1()
                    .px_2()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("back")
                            .ghost()
                            .icon(IconName::ChevronLeft)
                            .accessibility_label("Back")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.show_main(window, cx)),
                            ),
                    )
                    .child(div().font_semibold().child("Locations")),
            )
            .child(SmoothScroll::new(
                "locations-scroll",
                self.locations.read(cx).scroll_handle(),
                List::new(&self.locations)
                    .flex_1()
                    .min_h_0()
                    .pb_2()
                    .search_placeholder("Search locations"),
            ))
    }
}

impl Render for Popover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let motion = cx.theme().motion_tokens();
        let (entered, moving) = self.presence.shown(cx.background_executor().now(), motion);
        if moving {
            window.request_animation_frame();
        }
        let travel = if cx.reduce_motion() {
            px(0.)
        } else {
            motion.distance_medium.to_pixels(window.rem_size()) * (1. - entered)
        };
        let (left, top) = match self.toward_icon {
            Some(Side::Top) => (px(0.), -travel),
            Some(Side::Bottom) => (px(0.), travel),
            Some(Side::Left) => (-travel, px(0.)),
            Some(Side::Right) => (travel, px(0.)),
            None => (px(0.), px(0.)),
        };

        let shown = match self.page {
            Page::Main => 0.,
            Page::Locations => 1.,
        };
        let policy = Transition::new(motion.duration_normal).easing(motion.easing_move.clone());
        // Both pages sit side by side on a track that slides one card width per page.
        let progress: f32 = transition(("popover", "page"), shown, policy, window, cx);
        let page = || div().w(CARD.width).h_full().flex_none();
        // The page being shown is mounted from the first frame of the slide, so it can take focus.
        let main_mounted = self.page == Page::Main || progress < 1.;
        let locations_mounted = self.page == Page::Locations || progress > 0.;
        let globe = main_mounted.then(|| self.globe.render(window, cx));

        let fill = match self.surface {
            Surface::Blurred => theme::translucent_popover(cx),
            Surface::Solid => cx.theme().popover,
        };

        // The window, its OS shadow and backdrop show at once and stay put; only the content fades
        // and slides in, which also keeps the backdrop from showing untinted.
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &Cancel, window, cx| this.dismiss(window, cx)))
            .on_action(cx.listener(|this, _: &Escape, window, cx| this.dismiss(window, cx)))
            .size_full()
            .bg(fill)
            .text_color(cx.theme().popover_foreground)
            .rounded(cx.theme().radius_lg)
            .overflow_hidden()
            .child(
                h_flex()
                    .items_start()
                    .h_full()
                    .w(CARD.width * 2.)
                    .relative()
                    .left(left - CARD.width * progress)
                    .top(top)
                    .opacity(entered)
                    .child(page().when_some(globe, |page, globe| {
                        page.relative().child(globe).child(self.render_main(cx))
                    }))
                    .child(page().when(locations_mounted, |page| {
                        page.child(self.render_locations(cx))
                    })),
            )
    }
}

/// The locations VyprVPN listed when the fixture was recorded.
#[cfg(any(debug_assertions, test))]
fn recorded_locations() -> std::result::Result<Vec<ipc::Location>, ipc::CodecError> {
    #[derive(serde::Deserialize)]
    struct LocationsResponse {
        locations: Vec<ipc::Location>,
    }

    let response: LocationsResponse = ipc::decode(include_str!(
        "../../../vypr-api/tests/fixtures/locations.json"
    ))?;

    Ok(response.locations)
}

/// The country's cached flag, or a neutral placeholder of the same size while it is missing.
fn flag(flag_dir: Option<&Path>, country_code: &str, cx: &App) -> AnyElement {
    let muted = cx.theme().muted;
    let placeholder = move || {
        div()
            .w_5()
            .h_3p5()
            .rounded_xs()
            .bg(muted)
            .into_any_element()
    };
    // GPUI remembers a failed image load for the life of the app, so a flag still downloading
    // must not be loaded yet.
    let path = flag_dir
        .and_then(|dir| Some(dir.join(flags::file_name(country_code).ok()?)))
        .filter(|path| path.is_file());

    h_flex()
        .size_5()
        .flex_none()
        .justify_center()
        .child(match path {
            Some(path) => img(path)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .with_fallback(placeholder)
                .into_any_element(),
            None => placeholder(),
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use gpui_kit::component::{ActiveTheme, IndexPath};
    use gpui_kit::{AppContext, Empty, Entity, TestAppContext, VisualTestContext};
    use ipc::Status;

    use gpui_kit::component::list::ListDelegate;

    use super::{Page, Popover, PopoverEvent, init, recorded_locations};
    use crate::helper_link::{HelperState, LinkState, Locations};
    use crate::popover::model::tests::state;

    fn disconnected() -> LinkState {
        let mut state = state(
            Status::Disconnected {
                last_spoke_id: Some("uk1".into()),
                error: None,
            },
            None,
        );
        state.locations = Locations::Loaded(recorded_locations().unwrap());
        state
    }

    type Events = Rc<RefCell<Vec<PopoverEvent>>>;

    fn exit_duration(cx: &mut VisualTestContext) -> Duration {
        cx.update(|_, cx| cx.theme().motion_tokens().duration_fast)
    }

    fn open(
        state: LinkState,
        cx: &mut TestAppContext,
    ) -> (Entity<Popover>, Events, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
        });
        let (popover, cx) = cx.add_window_view(|window, cx| Popover::new(state, None, window, cx));

        let events = Events::default();
        cx.update({
            let events = events.clone();
            |_, cx| {
                cx.subscribe(&popover, move |_, event: &PopoverEvent, _| {
                    events.borrow_mut().push(event.clone());
                })
                .detach();
            }
        });

        (popover, events, cx)
    }

    fn listed(popover: &Entity<Popover>, cx: &mut VisualTestContext) -> Vec<String> {
        popover.read_with(cx, |popover, cx| {
            let list = popover.locations.read(cx);
            let delegate = list.delegate();

            (0..delegate.sections_count(cx))
                .flat_map(|section| {
                    (0..delegate.items_count(section, cx))
                        .map(move |row| IndexPath::new(row).section(section))
                })
                .filter_map(|index| delegate.spoke_id(index).map(str::to_owned))
                .collect()
        })
    }

    fn page(popover: &Entity<Popover>, cx: &mut VisualTestContext) -> Page {
        popover.read_with(cx, |popover, _| popover.page)
    }

    #[gpui_kit::test]
    fn searching_narrows_the_list_and_enter_picks_the_match(cx: &mut TestAppContext) {
        let (popover, events, cx) = open(disconnected(), cx);

        for (query, spoke_id) in [("perth", "au3"), ("GER", "de1"), ("de1", "de1")] {
            popover.update_in(cx, |popover, window, cx| popover.show_locations(window, cx));
            cx.run_until_parked();
            assert_eq!(listed(&popover, cx).len(), 73);

            cx.simulate_input(query);
            cx.run_until_parked();
            assert_eq!(listed(&popover, cx), [spoke_id], "searching {query:?}");

            cx.simulate_keystrokes("enter");
            cx.run_until_parked();
            assert_eq!(page(&popover, cx), Page::Main);
            assert_eq!(
                events.borrow_mut().pop(),
                Some(PopoverEvent::Select(spoke_id.into()))
            );

            popover.update(cx, |popover, cx| popover.press_connect(cx));
            assert_eq!(
                events.borrow_mut().pop(),
                Some(PopoverEvent::Connect(spoke_id.into()))
            );
        }
    }

    #[gpui_kit::test]
    fn arrow_keys_move_from_the_chosen_location(cx: &mut TestAppContext) {
        let (popover, events, cx) = open(disconnected(), cx);
        popover.update_in(cx, |popover, window, cx| popover.show_locations(window, cx));
        cx.run_until_parked();

        let listed = listed(&popover, cx);
        let chosen = listed
            .iter()
            .position(|spoke_id| spoke_id == "uk1")
            .unwrap();

        cx.simulate_keystrokes("down down up enter");
        cx.run_until_parked();

        assert_eq!(
            *events.borrow(),
            [PopoverEvent::Select(listed[chosen + 1].clone())]
        );
    }

    #[gpui_kit::test]
    fn escape_steps_back_from_the_subpage_then_closes(cx: &mut TestAppContext) {
        let (popover, events, cx) = open(disconnected(), cx);
        popover.update_in(cx, |popover, window, cx| popover.show_locations(window, cx));
        cx.run_until_parked();

        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(page(&popover, cx), Page::Main);
        assert!(events.borrow().is_empty());

        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1, "the card fades out first");

        let exit = exit_duration(cx);
        cx.executor().advance_clock(exit);
        cx.run_until_parked();
        assert!(cx.windows().is_empty());
    }

    #[gpui_kit::test]
    fn losing_focus_closes_the_popover(cx: &mut TestAppContext) {
        let (_popover, _events, cx) = open(disconnected(), cx);
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        assert_eq!(cx.windows().len(), 1);

        cx.deactivate_window();
        let exit = exit_duration(cx);
        cx.executor().advance_clock(exit);
        cx.run_until_parked();

        assert!(cx.windows().is_empty());
    }

    #[gpui_kit::test]
    fn using_the_popover_keeps_it_open(cx: &mut TestAppContext) {
        let (popover, events, cx) = open(disconnected(), cx);
        let popover_window = cx.update(|window, _| window.window_handle());
        let other_window = cx.update(|_, cx| {
            cx.open_window(Default::default(), |_, cx| cx.new(|_| Empty))
                .unwrap()
        });
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();

        // How Windows can report the click that brings the popover to the foreground: inactive,
        // then active again, both before either is handled.
        cx.update(|window, cx| {
            other_window
                .update(cx, |_, other, _| other.activate_window())
                .unwrap();
            window.activate_window();
        });
        cx.run_until_parked();

        popover.update_in(cx, |popover, window, cx| popover.show_locations(window, cx));
        cx.run_until_parked();
        cx.simulate_input("perth");
        cx.simulate_keystrokes("enter");
        popover.update(cx, |popover, cx| popover.press_connect(cx));
        cx.run_until_parked();

        assert!(cx.windows().contains(&popover_window));
        assert_eq!(page(&popover, cx), Page::Main);
        assert_eq!(
            *events.borrow(),
            [
                PopoverEvent::Select("au3".into()),
                PopoverEvent::Connect("au3".into())
            ]
        );
    }

    #[gpui_kit::test]
    fn the_button_follows_the_status_it_is_handed(cx: &mut TestAppContext) {
        let (popover, events, cx) = open(disconnected(), cx);
        popover.update(cx, |popover, cx| popover.press_connect(cx));

        let mut connected = disconnected();
        connected.helper = HelperState::Running(Status::Connected {
            spoke_id: "uk1".into(),
            tunnel_ip: [10, 1, 2, 3].into(),
            connected_at: 1_760_140_800,
        });
        popover.update(cx, |popover, cx| {
            popover.set_state(connected, cx);
            popover.press_connect(cx);
        });

        assert_eq!(
            *events.borrow(),
            [
                PopoverEvent::Connect("uk1".into()),
                PopoverEvent::Disconnect
            ]
        );
    }
}
