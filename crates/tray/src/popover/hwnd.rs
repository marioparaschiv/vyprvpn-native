use gpui_kit::Window;
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle};
use winsafe::{EnumThreadWindows, GetCurrentThreadId, HWND, co};

#[derive(Debug, thiserror::Error)]
pub enum HwndError {
    #[error("the popover has no window handle: {0}")]
    Handle(#[from] HandleError),
    #[error("the popover is not a Win32 window")]
    NotWin32,
    #[error("listing this thread's windows failed: {0}")]
    Enumerate(#[from] co::ERROR),
    #[error("the popover is not one of this thread's windows")]
    NotFound,
}

/// The Win32 window behind `window`.
pub fn of(window: &Window) -> Result<HWND, HwndError> {
    let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window)?.as_raw() else {
        return Err(HwndError::NotWin32);
    };

    // winsafe can't wrap a raw HWND without unsafe, so find the same window among ours.
    let mut found = None;
    EnumThreadWindows(GetCurrentThreadId(), |hwnd: HWND| {
        if hwnd.ptr() as isize == handle.hwnd.get() {
            found = Some(hwnd);
        }
        true
    })?;

    found.ok_or(HwndError::NotFound)
}
