//! What a left click on the tray icon does to the popover.

use std::time::{Duration, Instant};

/// Pressing the tray icon takes focus from the popover, which closes it before the click arrives.
/// A click this soon after the popover closed is that same click, so it must not reopen it.
const CLOSING_CLICK: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Open,
    Close,
    /// The click already closed the popover.
    Nothing,
}

/// `closed_at` is when the popover last closed; times come from GPUI's clock.
pub fn on_click(open: bool, closed_at: Option<Instant>, now: Instant) -> Toggle {
    if open {
        return Toggle::Close;
    }

    match closed_at {
        Some(closed_at) if now.saturating_duration_since(closed_at) < CLOSING_CLICK => {
            Toggle::Nothing
        }
        _ => Toggle::Open,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_opens_the_closed_popover() {
        let now = Instant::now();

        assert_eq!(on_click(false, None, now), Toggle::Open);
        assert_eq!(
            on_click(false, Some(now - Duration::from_secs(5)), now),
            Toggle::Open
        );
    }

    #[test]
    fn a_second_click_closes_the_open_popover() {
        let now = Instant::now();

        assert_eq!(on_click(true, None, now), Toggle::Close);
    }

    #[test]
    fn the_click_that_took_focus_from_the_popover_leaves_it_closed() {
        let now = Instant::now();

        assert_eq!(
            on_click(false, Some(now - Duration::from_millis(120)), now),
            Toggle::Nothing
        );
        assert_eq!(on_click(false, Some(now), now), Toggle::Nothing);
        assert_eq!(
            on_click(false, Some(now - CLOSING_CLICK), now),
            Toggle::Open
        );
    }
}
