//! What shows behind the card: the OS's blur of the windows beneath it where the OS offers one and
//! the user allows transparency, otherwise the theme's solid surface.

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub use windows::apply;

use gpui_kit::WindowBackgroundAppearance;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Surface {
    /// The OS blurs what is behind the window and the card tints it.
    Blurred,
    #[default]
    Solid,
}

impl Surface {
    /// Read on each open: the popover closes as soon as the user turns to the setting, so it never
    /// needs to follow a change while it is open.
    #[cfg(target_os = "windows")]
    pub fn current() -> Self {
        windows::surface()
            .inspect_err(|error| log::warn!("showing the solid card: {error}"))
            .unwrap_or_default()
    }

    /// AppKit's blur turns itself opaque when the user reduces transparency.
    #[cfg(target_os = "macos")]
    pub fn current() -> Self {
        Self::Blurred
    }

    pub fn window_background(self) -> WindowBackgroundAppearance {
        match self {
            #[cfg(target_os = "macos")]
            Self::Blurred => WindowBackgroundAppearance::Blurred,
            // GPUI's Windows blur is the legacy accent policy, which DWM draws over any system
            // backdrop. Mica is the mode that leaves the backdrop to DWM, and [`apply`] then
            // swaps Mica for the acrylic Windows 11 flyouts use.
            #[cfg(target_os = "windows")]
            Self::Blurred => WindowBackgroundAppearance::MicaBackdrop,
            Self::Solid => WindowBackgroundAppearance::Transparent,
        }
    }
}
