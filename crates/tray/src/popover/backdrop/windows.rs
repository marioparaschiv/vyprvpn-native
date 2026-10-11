use windows::UI::ViewManagement::UISettings;
use windows_version::OsVersion;
use winsafe::{DwmAttr, HWND, co};

use super::Surface;

/// The first Windows 11 build, which rounds a window's corners on request.
const ROUNDED_CORNERS: u32 = 22000;
/// Windows 11 22H2, the first to offer its acrylic as a window backdrop.
const ACRYLIC_BACKDROP: u32 = 22621;

#[derive(Debug, thiserror::Error)]
pub enum BackdropError {
    #[error("reading the transparency effects setting failed: {0}")]
    Settings(#[from] ::windows::core::Error),
    #[error("DWM refused the window's corners or backdrop: {0}")]
    Dwm(#[from] co::HRESULT),
}

pub fn surface() -> Result<Surface, BackdropError> {
    Ok(choose(
        OsVersion::current().build,
        UISettings::new()?.AdvancedEffectsEnabled()?,
    ))
}

/// Transparency effects off covers Energy Saver too, which turns the setting off while it runs.
fn choose(build: u32, transparency_effects: bool) -> Surface {
    if build >= ACRYLIC_BACKDROP && transparency_effects {
        Surface::Blurred
    } else {
        Surface::Solid
    }
}

/// Rounds the window's corners, which also gives it the system's border and shadow, and puts the
/// acrylic behind a [`Surface::Blurred`] card.
pub fn apply(popover: &HWND, surface: Surface) -> Result<(), BackdropError> {
    if OsVersion::current().build < ROUNDED_CORNERS {
        return Ok(());
    }

    popover.DwmSetWindowAttribute(DwmAttr::WindowCornerPreference(co::DWMWCP::ROUND))?;
    if surface == Surface::Blurred {
        popover.DwmSetWindowAttribute(DwmAttr::SystemBackdropType(co::DWMSBT::TRANSIENTWINDOW))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blurs_only_where_windows_offers_acrylic_and_transparency_is_on() {
        let cases = [
            (19045, true, Surface::Solid),
            (22000, true, Surface::Solid),
            (22621, true, Surface::Blurred),
            (26100, true, Surface::Blurred),
            (26100, false, Surface::Solid),
        ];

        for (build, transparency_effects, expected) in cases {
            assert_eq!(
                choose(build, transparency_effects),
                expected,
                "build {build}, transparency effects {transparency_effects}"
            );
        }
    }
}
