use winsafe::HWND;

#[derive(Debug, thiserror::Error)]
#[error("Windows refused to bring the popover to the foreground")]
pub struct Refused;

/// Brings `popover` to the foreground with `SetForegroundWindow`, which the tray click entitles
/// this process to. GPUI's own `activate_window` sends a synthetic Alt press first, which can
/// open the menu bar of the app that had focus.
pub fn take(popover: &HWND) -> Result<(), Refused> {
    if !popover.SetForegroundWindow() {
        return Err(Refused);
    }

    Ok(())
}
