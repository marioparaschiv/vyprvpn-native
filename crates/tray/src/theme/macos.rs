use gpui_kit::{Hsla, Rgba};
use objc2_app_kit::{NSColor, NSColorSpace};

#[derive(Debug, thiserror::Error)]
#[error("the accent colour has no sRGB form")]
pub struct AccentError;

pub fn accent_color() -> Result<Hsla, AccentError> {
    let color = NSColor::controlAccentColor()
        .colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())
        .ok_or(AccentError)?;

    Ok(Rgba {
        r: color.redComponent() as f32,
        g: color.greenComponent() as f32,
        b: color.blueComponent() as f32,
        a: color.alphaComponent() as f32,
    }
    .into())
}
