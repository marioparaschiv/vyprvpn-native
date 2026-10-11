#[cfg(not(target_os = "macos"))]
mod lock;
#[cfg(target_os = "macos")]
mod symbol;

#[cfg(not(target_os = "macos"))]
pub use lock::icon;
#[cfg(target_os = "macos")]
pub use symbol::icon;

use tray_icon::BadIcon;

/// The connected colour, shared by the Windows tray lock and the popover's globe marker.
pub const CONNECTED: [u8; 3] = [0x8E, 0xE0, 0xA1];

/// Which tray icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Disconnected,
    Busy,
    Connected,
}

#[derive(Debug, thiserror::Error)]
pub enum IconError {
    #[error(transparent)]
    Bad(#[from] BadIcon),
    #[cfg(target_os = "macos")]
    #[error("the SF Symbol {0} is unavailable")]
    MissingSymbol(&'static str),
    #[cfg(target_os = "macos")]
    #[error("AppKit could not make a bitmap to draw the SF Symbol in")]
    NoBitmap,
}
