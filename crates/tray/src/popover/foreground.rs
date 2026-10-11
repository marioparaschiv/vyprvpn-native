use gpui_kit::Window;
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle};
use winsafe::{EnumThreadWindows, GetCurrentThreadId, HWND, co};

#[derive(Debug, thiserror::Error)]
pub enum ForegroundError {
    #[error("the popover has no window handle: {0}")]
    Handle(#[from] HandleError),
    #[error("the popover is not a Win32 window")]
    NotWin32,
    #[error("listing this thread's windows failed: {0}")]
    Enumerate(#[from] co::ERROR),
    #[error("the popover is not one of this thread's windows")]
    NotFound,
    #[error("Windows refused to bring the popover to the foreground")]
    Refused,
}

/// Brings `window` to the foreground with `SetForegroundWindow`, which the tray click entitles
/// this process to. GPUI's own `activate_window` sends a synthetic Alt press first, which can
/// open the menu bar of the app that had focus.
pub fn take(window: &Window) -> Result<(), ForegroundError> {
    let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window)?.as_raw() else {
        return Err(ForegroundError::NotWin32);
    };

    // winsafe can't wrap a raw HWND without unsafe, so find the same window among ours.
    let mut popover = None;
    EnumThreadWindows(GetCurrentThreadId(), |hwnd: HWND| {
        if hwnd.ptr() as isize == handle.hwnd.get() {
            popover = Some(hwnd);
        }
        true
    })?;
    let popover = popover.ok_or(ForegroundError::NotFound)?;

    if !popover.SetForegroundWindow() {
        return Err(ForegroundError::Refused);
    }

    Ok(())
}
