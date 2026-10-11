//! Keeps the gpui-kit theme in step with the OS: light or dark mode and the accent colour, switching
//! live when either changes.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use macos as os;
#[cfg(windows)]
use windows as os;

#[cfg(windows)]
use gpui_kit::AsyncApp;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, Subscription, Window};
#[cfg(windows)]
use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[cfg(windows)]
    #[error("following the OS colours failed: {0}")]
    Os(#[from] ::windows::core::Error),
}

/// Applies the OS colours now and again whenever the OS reports a change. Call once at startup.
///
/// macOS has no safe API for the app-wide change notification, so there the colours follow only
/// through [`observe_window`].
pub fn follow_system(cx: &mut App) -> Result<(), ThemeError> {
    apply(cx);

    #[cfg(windows)]
    follow_changes(cx)?;

    Ok(())
}

#[cfg(windows)]
fn follow_changes(cx: &mut App) -> Result<(), ThemeError> {
    let (changed, mut changes) = mpsc::unbounded_channel();
    let subscription = windows::on_change(move || {
        // Fails only once the app is quitting.
        changed.send(()).ok();
    })?;

    cx.spawn(async move |cx: &mut AsyncApp| {
        let _subscription = subscription;
        while changes.recv().await.is_some() {
            cx.update(apply);
        }
    })
    .detach();

    Ok(())
}

/// Re-applies the OS colours when `window` sees the appearance change, in case the OS notifies the
/// window before the app-wide subscription.
pub fn observe_window(window: &Window) -> Subscription {
    window.observe_window_appearance(|_, cx| apply(cx))
}

fn apply(cx: &mut App) {
    Theme::change(mode(cx), None, cx);

    match os::accent_color() {
        Ok(accent) => Theme::update(cx, |theme| {
            let (hover, active, foreground) = (
                shade(accent, 0.06),
                shade(accent, -0.06),
                foreground_on(accent),
            );

            theme.primary = accent;
            theme.primary_hover = hover;
            theme.primary_active = active;
            theme.primary_foreground = foreground;
            theme.button_primary = accent;
            theme.button_primary_hover = hover;
            theme.button_primary_active = active;
            theme.button_primary_foreground = foreground;
            theme.ring = accent;
            theme.selection = accent.opacity(0.35);
        }),
        Err(error) => log::warn!("reading the accent colour failed: {error}"),
    }
}

fn mode(cx: &App) -> ThemeMode {
    #[cfg(debug_assertions)]
    if let Some(mode) = dev_override() {
        return mode;
    }

    cx.window_appearance().into()
}

/// `VYPRLITE_THEME=light` or `dark` pins the mode in debug builds, for screenshots without touching
/// the OS setting.
#[cfg(debug_assertions)]
fn dev_override() -> Option<ThemeMode> {
    match std::env::var("VYPRLITE_THEME").ok()?.as_str() {
        "light" => Some(ThemeMode::Light),
        "dark" => Some(ThemeMode::Dark),
        other => {
            log::warn!("ignoring VYPRLITE_THEME={other:?}; use light or dark");
            None
        }
    }
}

fn shade(color: Hsla, lightness: f32) -> Hsla {
    Hsla {
        l: (color.l + lightness).clamp(0.0, 1.0),
        ..color
    }
}

/// White text on dark accents, black on light ones, as Windows itself does.
fn foreground_on(accent: Hsla) -> Hsla {
    let rgba = accent.to_rgb();
    let luminance = 0.299 * rgba.r + 0.587 * rgba.g + 0.114 * rgba.b;

    if luminance > 0.6 {
        gpui_kit::black()
    } else {
        gpui_kit::white()
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Rgba, black, white};

    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Hsla {
        Rgba {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: 1.0,
        }
        .into()
    }

    #[test]
    fn picks_readable_text_for_the_accent() {
        assert_eq!(foreground_on(rgb(0x00, 0x78, 0xD4)), white());
        assert_eq!(foreground_on(rgb(0x99, 0xEB, 0xFF)), black());
    }
}
