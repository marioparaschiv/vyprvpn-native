use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;
use ipc::{Credentials, Method};

use crate::helper_link::Requests;
use crate::theme;

const WIDTH: f32 = 320.;
/// A first guess; the window fits itself to its content once laid out.
const INITIAL_HEIGHT: f32 = 170.;

/// Asks for the VyprVPN username and password and logs in through the helper. Stays open, keeping
/// the username, until the login succeeds or the user cancels. Its height follows its content, so
/// it grows to fit an error.
pub struct LoginWindow {
    username: Entity<InputState>,
    password: Entity<InputState>,
    error: Option<SharedString>,
    logging_in: bool,
    requests: Requests,
    _subscriptions: Vec<Subscription>,
}

impl LoginWindow {
    pub fn open(cx: &mut App, requests: Requests) -> Result<AnyWindowHandle> {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(
                size(px(WIDTH), px(INITIAL_HEIGHT)),
                cx,
            )),
            titlebar: Some(TitlebarOptions {
                title: Some("Log in to VyprVPN".into()),
                ..Default::default()
            }),
            is_resizable: false,
            is_minimizable: false,
            ..Default::default()
        };

        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Self::new(requests, window, cx))
        })?;
        cx.activate(true);

        Ok(window)
    }

    fn new(requests: Requests, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let username = cx.new(|cx| InputState::new(window, cx).placeholder("Username"));
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Password")
                .masked(true)
        });
        let submit_on_enter = |this: &mut Self,
                               _: &Entity<InputState>,
                               event: &InputEvent,
                               window: &mut Window,
                               cx: &mut Context<Self>| {
            if let InputEvent::PressEnter { .. } = event {
                this.submit(window, cx);
            }
        };
        let _subscriptions = vec![
            cx.subscribe_in(&username, window, submit_on_enter),
            cx.subscribe_in(&password, window, submit_on_enter),
            theme::observe_window(window),
        ];
        username.update(cx, |input, cx| input.focus(window, cx));

        Self {
            username,
            password,
            error: dev_error(),
            logging_in: false,
            requests,
            _subscriptions,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.logging_in {
            return;
        }

        let credentials = Credentials {
            username: self.username.read(cx).value().trim().to_owned(),
            password: self.password.read(cx).value().to_string(),
        };
        if credentials.username.is_empty() || credentials.password.is_empty() {
            self.error = Some("Enter your username and password.".into());
            cx.notify();
            return;
        }

        self.logging_in = true;
        self.error = None;
        cx.notify();

        let requests = self.requests.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = requests.send(Method::Login(credentials)).await;

            // GPUI reports an update that removes its window as "window not found", and the user
            // may have closed the window meanwhile; either way there is nothing left to update.
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => window.remove_window(),
                Err(error) => {
                    this.logging_in = false;
                    this.error = Some(error.to_string().into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

impl Render for LoginWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = v_flex()
            .w_full()
            .p_5()
            .gap_3()
            .child(Input::new(&self.username).disabled(self.logging_in))
            .child(Input::new(&self.password).disabled(self.logging_in))
            .children(
                self.error
                    .clone()
                    .map(|error| div().text_sm().text_color(cx.theme().danger).child(error)),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel")
                            .label("Cancel")
                            .on_click(|_, window, _| window.remove_window()),
                    )
                    .child(
                        Button::new("log-in")
                            .primary()
                            .label("Log in")
                            .loading(self.logging_in)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            );

        div()
            .size_full()
            .bg(cx.theme().background)
            .on_action(cx.listener(|_, _: &Escape, window, _| window.remove_window()))
            .on_children_prepainted(fit_height)
            .child(content)
    }
}

/// `VYPRVPN_DEV_LOGIN_ERROR=<text>` opens the window showing that error in debug builds, for
/// screenshots without a failed login.
fn dev_error() -> Option<SharedString> {
    #[cfg(debug_assertions)]
    return std::env::var("VYPRVPN_DEV_LOGIN_ERROR")
        .ok()
        .map(Into::into);

    #[cfg(not(debug_assertions))]
    None
}

/// Resizes the window to the content's height once it is laid out.
fn fit_height(children: Vec<Bounds<Pixels>>, window: &mut Window, _: &mut App) {
    let Some(content) = children.first() else {
        return;
    };

    if (content.size.height - window.viewport_size().height).abs() >= px(1.) {
        window.resize(size(px(WIDTH), content.size.height));
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use ipc::{Credentials, Method};

    use super::LoginWindow;
    use crate::helper_link::{RequestError, Requests};

    #[gpui_kit::test]
    fn enter_submits_a_refusal_shows_inline_and_escape_cancels(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (requests, mut commands) = Requests::intercepted();
        let (login, cx) = cx.add_window_view(|window, cx| LoginWindow::new(requests, window, cx));

        cx.simulate_input("alice");
        login.update_in(cx, |login, window, cx| {
            login
                .password
                .update(cx, |input, cx| input.set_value("correct horse", window, cx));
        });
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();

        let command = commands.try_recv().unwrap();
        assert_eq!(
            command.method,
            Method::Login(Credentials {
                username: "alice".into(),
                password: "correct horse".into(),
            })
        );
        command.finish(Err(RequestError::Refused(
            "invalid username or password".into(),
        )));
        cx.run_until_parked();

        login.read_with(cx, |login, cx| {
            assert_eq!(login.error.as_deref(), Some("invalid username or password"));
            assert!(!login.logging_in);
            assert_eq!(login.username.read(cx).value(), "alice");
        });

        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(cx.windows().is_empty());
    }

    #[gpui_kit::test]
    fn closes_once_logged_in(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (requests, mut commands) = Requests::intercepted();
        let (login, cx) = cx.add_window_view(|window, cx| LoginWindow::new(requests, window, cx));
        login.update_in(cx, |login, window, cx| {
            for (input, value) in [
                (&login.username, "alice"),
                (&login.password, "correct horse"),
            ] {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
            login.submit(window, cx);
        });
        cx.run_until_parked();

        commands.try_recv().unwrap().finish(Ok(()));
        cx.run_until_parked();

        assert!(cx.windows().is_empty());
    }
}
