//! Where the popover window opens for a click on the tray icon, converted from the physical pixels
//! the click reports to the logical pixels GPUI places windows in.

use gpui_kit::{App, Bounds, DisplayId, Pixels, Point, point, px};
use placement::{LogicalSize, PhysicalRect};

use super::CARD;
use super::presence::{self, Side};

#[derive(Debug, thiserror::Error)]
pub enum AnchorError {
    #[cfg(windows)]
    #[error("reading the tray icon's monitor failed: {0}")]
    MonitorInfo(#[from] winsafe::co::ERROR),
    #[cfg(windows)]
    #[error("GPUI does not list the tray icon's monitor")]
    UnknownDisplay,
    #[cfg(target_os = "macos")]
    #[error("screens can only be read on the main thread")]
    NotMainThread,
    #[cfg(target_os = "macos")]
    #[error("no screen holds the pointer that clicked the menu bar icon")]
    NoScreen,
}

/// The monitor the tray icon is on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    /// The display to open the window on; `None` for the primary one.
    pub display: Option<DisplayId>,
    /// In physical pixels.
    pub bounds: PhysicalRect,
    /// In physical pixels.
    pub work_area: PhysicalRect,
    /// 1.5 at 150%.
    pub scale: f32,
}

/// Where the popover window opens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub display: Option<DisplayId>,
    pub origin: Point<Pixels>,
    /// The card's side facing the tray icon, which it enters from.
    pub toward_icon: Option<Side>,
}

impl Anchor {
    /// The middle of the primary display.
    pub fn centred(cx: &App) -> Self {
        Self {
            display: None,
            origin: Bounds::centered(None, CARD, cx).origin,
            toward_icon: None,
        }
    }
}

pub fn icon_rect(rect: tray_icon::Rect) -> PhysicalRect {
    PhysicalRect {
        x: rect.position.x.round() as i32,
        y: rect.position.y.round() as i32,
        width: rect.size.width as i32,
        height: rect.size.height as i32,
    }
}

/// Places the card where [`placement::popover_origin`] puts it. GPUI positions a window in logical pixels of the display it opens on, which are that
/// display's physical pixels divided by its scale.
pub fn anchor(icon: PhysicalRect, monitor: &Monitor) -> Anchor {
    let card = LogicalSize {
        width: CARD.width.as_f32(),
        height: CARD.height.as_f32(),
    };
    let card_origin =
        placement::popover_origin(icon, monitor.bounds, monitor.work_area, card, monitor.scale);
    let card_rect = PhysicalRect {
        x: card_origin.x,
        y: card_origin.y,
        width: (card.width * monitor.scale).ceil() as i32,
        height: (card.height * monitor.scale).ceil() as i32,
    };

    Anchor {
        display: monitor.display,
        origin: point(
            px(card_origin.x as f32 / monitor.scale),
            px(card_origin.y as f32 / monitor.scale),
        ),
        toward_icon: presence::facing(card_rect, icon),
    }
}

/// The monitor showing most of `icon`.
#[cfg(windows)]
pub fn monitor(icon: PhysicalRect, cx: &App) -> Result<Monitor, AnchorError> {
    use winsafe::{HMONITOR, RECT, co};

    let handle = HMONITOR::MonitorFromRect(
        RECT {
            left: icon.x,
            top: icon.y,
            right: icon.x + icon.width,
            bottom: icon.y + icon.height,
        },
        co::MONITOR::DEFAULTTONEAREST,
    );
    let info = handle.GetMonitorInfo()?;
    // GPUI's Windows platform identifies a display by its monitor handle.
    let display = DisplayId::new(handle.ptr().addr() as u64);
    let logical = cx
        .find_display(display)
        .ok_or(AnchorError::UnknownDisplay)?
        .bounds();
    let rect = |rect: RECT| PhysicalRect {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    };
    let bounds = rect(info.rcMonitor);

    Ok(Monitor {
        display: Some(display),
        bounds,
        work_area: rect(info.rcWork),
        scale: bounds.width as f32 / logical.size.width.as_f32(),
    })
}

/// The screen whose menu bar was just clicked: the one under the pointer.
#[cfg(target_os = "macos")]
pub fn monitor(_icon: PhysicalRect, _cx: &App) -> Result<Monitor, AnchorError> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSScreen};

    let main_thread = MainThreadMarker::new().ok_or(AnchorError::NotMainThread)?;
    let screens: Vec<Screen> = NSScreen::screens(main_thread)
        .iter()
        .map(|screen| {
            let (frame, visible) = (screen.frame(), screen.visibleFrame());

            Screen {
                frame: PointRect {
                    x: frame.origin.x,
                    y: frame.origin.y,
                    width: frame.size.width,
                    height: frame.size.height,
                },
                visible: PointRect {
                    x: visible.origin.x,
                    y: visible.origin.y,
                    width: visible.size.width,
                    height: visible.size.height,
                },
                scale: screen.backingScaleFactor(),
            }
        })
        .collect();
    let pointer = NSEvent::mouseLocation();

    menu_bar_monitor((pointer.x, pointer.y), &screens).ok_or(AnchorError::NoScreen)
}

/// A rectangle in AppKit points, measured from the bottom-left of the primary screen.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct PointRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct Screen {
    frame: PointRect,
    /// Without the menu bar and the Dock.
    visible: PointRect,
    scale: f64,
}

/// The screen holding `pointer`, in AppKit points. tray-icon reports the icon in points from the
/// primary screen's top-left, multiplied by its screen's scale, so the monitor is given in the same
/// units. Screens of different scales overlap in those units, which is why the pointer, not the
/// icon, picks the screen. GPUI places a window opened without a display in points from the
/// primary screen's top-left, which is what [`anchor`] turns these units back into.
#[cfg(any(target_os = "macos", test))]
fn menu_bar_monitor(pointer: (f64, f64), screens: &[Screen]) -> Option<Monitor> {
    let primary_height = screens.first()?.frame.height;
    let screen = screens.iter().find(|screen| {
        let frame = screen.frame;
        (frame.x..frame.x + frame.width).contains(&pointer.0)
            && (frame.y..=frame.y + frame.height).contains(&pointer.1)
    })?;
    let physical = |rect: PointRect| PhysicalRect {
        x: (rect.x * screen.scale).round() as i32,
        y: ((primary_height - rect.y - rect.height) * screen.scale).round() as i32,
        width: (rect.width * screen.scale).round() as i32,
        height: (rect.height * screen.scale).round() as i32,
    };

    Some(Monitor {
        display: None,
        bounds: physical(screen.frame),
        work_area: physical(screen.visible),
        scale: screen.scale as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> PhysicalRect {
        PhysicalRect {
            x,
            y,
            width,
            height,
        }
    }

    fn monitor(bounds: PhysicalRect, work_area: PhysicalRect, scale: f32) -> Monitor {
        Monitor {
            display: Some(DisplayId::new(1)),
            bounds,
            work_area,
            scale,
        }
    }

    /// The card's top-left in physical pixels, as GPUI puts it on screen.
    fn card_on_screen(anchor: Anchor, scale: f32) -> (f32, f32) {
        (
            anchor.origin.x.as_f32() * scale,
            anchor.origin.y.as_f32() * scale,
        )
    }

    #[test]
    fn converts_the_card_position_to_logical_pixels_at_each_scale() {
        // A 1920x1080 monitor with the taskbar at the bottom, 48 logical pixels high.
        let cases = [
            (1.0, rect(0, 0, 1920, 1032), rect(1500, 1032, 24, 48)),
            (1.25, rect(0, 0, 1920, 1020), rect(1500, 1020, 30, 60)),
            (1.5, rect(0, 0, 1920, 1008), rect(1500, 1008, 36, 72)),
        ];

        for (scale, work_area, icon) in cases {
            let monitor = monitor(rect(0, 0, 1920, 1080), work_area, scale);
            let anchor = anchor(icon, &monitor);
            let (x, y) = card_on_screen(anchor, scale);
            let card_height = CARD.height.as_f32() * scale;

            assert!(
                (y + card_height - work_area.height as f32).abs() < 1.,
                "scale {scale}: the card's bottom {} meets the taskbar at {}",
                y + card_height,
                work_area.height
            );
            let icon_centre = (icon.x + icon.width / 2) as f32;
            let card_centre = x + CARD.width.as_f32() * scale / 2.;
            assert!(
                (card_centre - icon_centre).abs() < 1.,
                "scale {scale}: the card is centred on {card_centre}, the icon on {icon_centre}"
            );
        }
    }

    #[test]
    fn places_exactly_at_150_percent() {
        let monitor = monitor(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1008), 1.5);

        assert_eq!(
            anchor(rect(1500, 1008, 36, 72), &monitor),
            Anchor {
                display: Some(DisplayId::new(1)),
                // The card at physical (1263, 378) is logical (842, 252).
                origin: point(px(842.), px(252.)),
                toward_icon: Some(Side::Bottom),
            }
        );
    }

    #[test]
    fn secondary_monitor_left_of_and_above_the_primary() {
        let monitor = monitor(
            rect(-2560, -360, 2560, 1440),
            rect(-2560, -360, 2560, 1368),
            1.25,
        );
        let anchor = anchor(rect(-800, 1008, 30, 60), &monitor);
        let (x, y) = card_on_screen(anchor, 1.25);

        assert!((-2560. ..0.).contains(&x), "x {x}");
        assert!(
            (y + CARD.height.as_f32() * 1.25 - 1008.).abs() < 1.,
            "y {y}"
        );
    }

    #[test]
    fn icon_rect_rounds_the_reported_position() {
        let reported = tray_icon::Rect {
            position: tray_icon::dpi::PhysicalPosition::new(1699.6, -0.4),
            size: tray_icon::dpi::PhysicalSize::new(24, 48),
        };

        assert_eq!(icon_rect(reported), rect(1700, 0, 24, 48));
    }

    fn points(x: f64, y: f64, width: f64, height: f64) -> PointRect {
        PointRect {
            x,
            y,
            width,
            height,
        }
    }

    /// A Retina built-in display as the primary screen, with a 1x external display above and to
    /// its right; each has a 25-point menu bar.
    fn macos_screens() -> [Screen; 2] {
        [
            Screen {
                frame: points(0., 0., 1440., 900.),
                visible: points(0., 0., 1440., 875.),
                scale: 2.,
            },
            Screen {
                frame: points(1440., 900., 1920., 1080.),
                visible: points(1440., 900., 1920., 1055.),
                scale: 1.,
            },
        ]
    }

    #[test]
    fn macos_opens_below_the_icon_on_the_built_in_display() {
        let monitor = menu_bar_monitor((1210., 890.), &macos_screens()).unwrap();
        assert_eq!(
            monitor,
            Monitor {
                display: None,
                bounds: rect(0, 0, 2880, 1800),
                work_area: rect(0, 50, 2880, 1750),
                scale: 2.,
            }
        );

        // A 22-point icon at x = 1200 points, as tray-icon reports it at 2x.
        let anchor = anchor(rect(2400, 0, 44, 50), &monitor);
        assert_eq!(anchor.origin, point(px(1211.) - CARD.width / 2., px(25.)));
    }

    #[test]
    fn macos_opens_below_the_icon_on_an_external_display() {
        let monitor = menu_bar_monitor((3010., 1975.), &macos_screens()).unwrap();
        assert_eq!(
            monitor,
            Monitor {
                display: None,
                bounds: rect(1440, -1080, 1920, 1080),
                work_area: rect(1440, -1055, 1920, 1055),
                scale: 1.,
            }
        );

        // Points from the primary screen's top-left: the external menu bar's bottom is 1055 points
        // above the primary screen's top.
        let anchor = anchor(rect(3000, -1080, 30, 25), &monitor);
        assert_eq!(
            anchor.origin,
            point(px(3015.) - CARD.width / 2., px(-1055.))
        );
    }

    #[test]
    fn macos_finds_no_screen_for_a_pointer_off_every_screen() {
        assert_eq!(menu_bar_monitor((-10., 10.), &macos_screens()), None);
        assert_eq!(menu_bar_monitor((10., 10.), &[]), None);
    }
}
