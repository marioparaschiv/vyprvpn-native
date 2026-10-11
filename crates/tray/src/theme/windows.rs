use gpui_kit::{Hsla, Rgba};
use windows::Foundation::TypedEventHandler;
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::core::{IInspectable, Result};

/// Keeps the OS change notification registered while it lives.
pub struct Subscription {
    settings: UISettings,
    token: i64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Err(error) = self.settings.RemoveColorValuesChanged(self.token) {
            log::warn!("unsubscribing from OS colour changes failed: {error}");
        }
    }
}

pub fn accent_color() -> Result<Hsla> {
    let color = UISettings::new()?.GetColorValue(UIColorType::Accent)?;

    Ok(Rgba {
        r: f32::from(color.R) / 255.0,
        g: f32::from(color.G) / 255.0,
        b: f32::from(color.B) / 255.0,
        a: f32::from(color.A) / 255.0,
    }
    .into())
}

/// Calls `changed`, on a background thread, whenever the light/dark mode or the accent colour
/// changes.
pub fn on_change(changed: impl Fn() + Send + 'static) -> Result<Subscription> {
    let settings = UISettings::new()?;
    let token = settings.ColorValuesChanged(
        &TypedEventHandler::<UISettings, IInspectable>::new(move |_, _| {
            changed();
            Ok(())
        }),
    )?;

    Ok(Subscription { settings, token })
}
