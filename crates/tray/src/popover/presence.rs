//! The popover's entrance and exit. GPUI can neither scale nor blur an element tree (only images
//! and SVGs take a transform, and there is no filter), so the card fades in while sliding a short
//! way out of the tray icon, and fades out on close.

use std::time::{Duration, Instant};

use gpui_kit::component::MotionTokens;
use placement::PhysicalRect;

/// A side of the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

/// The side of the card facing the tray icon, or `None` when the icon is not beside the card.
pub fn facing(card: PhysicalRect, icon: PhysicalRect) -> Option<Side> {
    let x = icon.x + icon.width / 2;
    let y = icon.y + icon.height / 2;

    if y >= card.y + card.height {
        Some(Side::Bottom)
    } else if y <= card.y {
        Some(Side::Top)
    } else if x >= card.x + card.width {
        Some(Side::Right)
    } else if x <= card.x {
        Some(Side::Left)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Presence {
    Entering {
        since: Instant,
    },
    /// `from` is how far in the card was when the exit began.
    Exiting {
        since: Instant,
        from: f32,
    },
}

impl Presence {
    /// How far in the card is at `now`, from 0 (hidden at the icon) to 1 (at rest), and whether it
    /// is still moving. Both directions ease out: the exit is shorter, not slower to start.
    pub fn shown(self, now: Instant, motion: &MotionTokens) -> (f32, bool) {
        let progress = |since: Instant, duration: Duration| {
            let elapsed = now.saturating_duration_since(since);
            if elapsed >= duration {
                return (1., false);
            }

            let linear = elapsed.as_secs_f32() / duration.as_secs_f32();
            (motion.easing_enter.sample(linear).clamp(0., 1.), true)
        };

        match self {
            Self::Entering { since } => progress(since, motion.duration_normal),
            Self::Exiting { since, from } => {
                let (eased, moving) = progress(since, motion.duration_fast);
                (from * (1. - eased), moving)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD: PhysicalRect = PhysicalRect {
        x: 1000,
        y: 400,
        width: 340,
        height: 420,
    };

    fn icon(x: i32, y: i32) -> PhysicalRect {
        PhysicalRect {
            x,
            y,
            width: 24,
            height: 24,
        }
    }

    #[test]
    fn the_card_grows_out_of_the_side_facing_the_icon() {
        // Windows taskbar at the bottom, card above the icon and pushed left by the screen edge.
        assert_eq!(facing(CARD, icon(1300, 830)), Some(Side::Bottom));
        // macOS menu bar, card centred under the icon.
        assert_eq!(facing(CARD, icon(1158, 370)), Some(Side::Top));
        // Taskbars on the left and right.
        assert_eq!(facing(CARD, icon(970, 600)), Some(Side::Left));
        assert_eq!(facing(CARD, icon(1350, 600)), Some(Side::Right));
        // No icon beside the card: the card was centred on the screen instead.
        assert_eq!(facing(CARD, icon(1158, 600)), None);
    }

    /// Samples a phase every 4 ms until it stops moving.
    fn samples(presence: Presence, start: Instant) -> Vec<f32> {
        let motion = MotionTokens::default();
        let mut values = Vec::new();

        for frame in 0.. {
            let (shown, moving) = presence.shown(start + Duration::from_millis(frame * 4), &motion);
            values.push(shown);
            if !moving {
                break;
            }
        }

        values
    }

    #[test]
    fn the_entrance_rises_from_hidden_to_rest_without_overshoot() {
        let start = Instant::now();
        let values = samples(Presence::Entering { since: start }, start);

        assert_eq!(values.first(), Some(&0.));
        assert_eq!(values.last(), Some(&1.));
        assert!(
            values.windows(2).all(|pair| pair[1] >= pair[0]),
            "{values:?}"
        );
        assert!(values.iter().all(|value| (0. ..=1.).contains(value)));
        // A strong ease-out: most of the way there in the first third.
        assert!(values[15] > 0.8, "{values:?}");
        assert_eq!(values.len(), 46, "180 ms at 4 ms a frame");
    }

    #[test]
    fn the_exit_is_shorter_and_falls_from_where_the_card_was() {
        let start = Instant::now();
        let values = samples(
            Presence::Exiting {
                since: start,
                from: 0.6,
            },
            start,
        );

        assert_eq!(values.first(), Some(&0.6));
        assert_eq!(values.last(), Some(&0.));
        assert!(
            values.windows(2).all(|pair| pair[1] <= pair[0]),
            "{values:?}"
        );
        assert_eq!(values.len(), 31, "120 ms at 4 ms a frame");
    }
}
