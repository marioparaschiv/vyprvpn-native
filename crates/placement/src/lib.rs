//! Where the popover opens relative to the tray icon, on Windows and macOS.

/// A point in physical (device) pixels on the virtual screen. Secondary monitors left of or
/// above the primary one have negative coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalPoint {
    pub x: i32,
    pub y: i32,
}

/// A rectangle in physical (device) pixels on the virtual screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PhysicalRect {
    fn right(self) -> i32 {
        self.x + self.width
    }

    fn bottom(self) -> i32 {
        self.y + self.height
    }

    fn centre(self) -> PhysicalPoint {
        PhysicalPoint {
            x: self.x + self.width / 2,
            y: self.y + self.height / 2,
        }
    }
}

/// A size in logical (scaled) pixels, the unit the UI is laid out in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// The popover's top-left corner: against the icon on the side facing away from the screen
/// edge the taskbar or menu bar is on, centred on the icon along that edge, and kept inside the
/// work area. `scale` is the monitor's scale factor (1.5 at 150%).
pub fn popover_origin(
    icon: PhysicalRect,
    monitor: PhysicalRect,
    work_area: PhysicalRect,
    popover: LogicalSize,
    scale: f32,
) -> PhysicalPoint {
    let width = (popover.width * scale).ceil() as i32;
    let height = (popover.height * scale).ceil() as i32;
    let centred_x = icon.centre().x - width / 2;
    let centred_y = icon.centre().y - height / 2;

    let (x, y) = match bar_edge(icon, monitor, work_area) {
        Edge::Top => (centred_x, icon.bottom()),
        Edge::Bottom => (centred_x, icon.y - height),
        Edge::Left => (icon.right(), centred_y),
        Edge::Right => (icon.x - width, centred_y),
    };

    PhysicalPoint {
        x: keep_within(x, work_area.x, work_area.right() - width),
        y: keep_within(y, work_area.y, work_area.bottom() - height),
    }
}

/// Like `clamp`, but a popover larger than the work area pins to its top or left instead of
/// panicking.
fn keep_within(value: i32, min: i32, max: i32) -> i32 {
    value.min(max).max(min)
}

fn bar_edge(icon: PhysicalRect, monitor: PhysicalRect, work_area: PhysicalRect) -> Edge {
    if icon.bottom() <= work_area.y {
        return Edge::Top;
    }

    if icon.y >= work_area.bottom() {
        return Edge::Bottom;
    }

    if icon.right() <= work_area.x {
        return Edge::Left;
    }

    if icon.x >= work_area.right() {
        return Edge::Right;
    }

    // The icon is inside the work area: it sits in the Windows 11 hidden-icons flyout, which opens
    // beside the taskbar, or the taskbar auto-hides and takes no work area.
    let insets = [
        (Edge::Top, work_area.y - monitor.y),
        (Edge::Bottom, monitor.bottom() - work_area.bottom()),
        (Edge::Left, work_area.x - monitor.x),
        (Edge::Right, monitor.right() - work_area.right()),
    ];
    if let Some((edge, _)) = insets
        .into_iter()
        .filter(|(_, inset)| *inset > 0)
        .max_by_key(|(_, inset)| *inset)
    {
        return edge;
    }

    let centre = icon.centre();
    let distances = [
        (Edge::Bottom, monitor.bottom() - centre.y),
        (Edge::Top, centre.y - monitor.y),
        (Edge::Left, centre.x - monitor.x),
        (Edge::Right, monitor.right() - centre.x),
    ];
    distances
        .into_iter()
        .min_by_key(|(_, distance)| *distance)
        .expect("four edges")
        .0
}

#[cfg(test)]
mod tests {
    use super::*;

    const POPOVER: LogicalSize = LogicalSize {
        width: 320.0,
        height: 400.0,
    };

    fn rect(x: i32, y: i32, width: i32, height: i32) -> PhysicalRect {
        PhysicalRect {
            x,
            y,
            width,
            height,
        }
    }

    fn point(x: i32, y: i32) -> PhysicalPoint {
        PhysicalPoint { x, y }
    }

    const MONITOR: PhysicalRect = PhysicalRect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn taskbar_at_the_bottom_opens_above_the_icon() {
        let work_area = rect(0, 0, 1920, 1032);
        let icon = rect(1700, 1032, 24, 48);

        assert_eq!(
            popover_origin(icon, MONITOR, work_area, POPOVER, 1.0),
            point(1552, 632)
        );
    }

    #[test]
    fn taskbar_at_the_top_opens_below_the_icon() {
        let work_area = rect(0, 48, 1920, 1032);
        let icon = rect(1700, 0, 24, 48);

        assert_eq!(
            popover_origin(icon, MONITOR, work_area, POPOVER, 1.0),
            point(1552, 48)
        );
    }

    #[test]
    fn taskbar_on_the_left_opens_right_of_the_icon() {
        let work_area = rect(48, 0, 1872, 1080);
        let icon = rect(0, 700, 48, 24);

        assert_eq!(
            popover_origin(icon, MONITOR, work_area, POPOVER, 1.0),
            point(48, 512)
        );
    }

    #[test]
    fn taskbar_on_the_right_opens_left_of_the_icon() {
        let work_area = rect(0, 0, 1872, 1080);
        let icon = rect(1872, 700, 48, 24);

        assert_eq!(
            popover_origin(icon, MONITOR, work_area, POPOVER, 1.0),
            point(1552, 512)
        );
    }

    #[test]
    fn icons_near_the_corners_keep_the_popover_in_the_work_area() {
        let bottom = rect(0, 0, 1920, 1032);
        let top = rect(0, 48, 1920, 1032);
        let left = rect(48, 0, 1872, 1080);
        let right = rect(0, 0, 1872, 1080);

        let cases = [
            (rect(1890, 1032, 24, 48), bottom, point(1600, 632)),
            (rect(4, 1032, 24, 48), bottom, point(0, 632)),
            (rect(1890, 0, 24, 48), top, point(1600, 48)),
            (rect(4, 0, 24, 48), top, point(0, 48)),
            (rect(0, 1050, 48, 24), left, point(48, 680)),
            (rect(0, 4, 48, 24), left, point(48, 0)),
            (rect(1872, 1050, 48, 24), right, point(1552, 680)),
            (rect(1872, 4, 48, 24), right, point(1552, 0)),
        ];
        for (icon, work_area, expected) in cases {
            assert_eq!(
                popover_origin(icon, MONITOR, work_area, POPOVER, 1.0),
                expected,
                "icon {icon:?}"
            );
        }
    }

    #[test]
    fn scales_the_popover_to_physical_pixels() {
        let cases = [
            (
                1.0,
                rect(0, 0, 1920, 1032),
                rect(1700, 1032, 24, 48),
                point(1552, 632),
            ),
            (
                1.25,
                rect(0, 0, 1920, 1020),
                rect(1700, 1020, 30, 60),
                point(1515, 520),
            ),
            (
                1.5,
                rect(0, 0, 1920, 1008),
                rect(1500, 1008, 36, 72),
                point(1278, 408),
            ),
        ];
        for (scale, work_area, icon, expected) in cases {
            assert_eq!(
                popover_origin(icon, MONITOR, work_area, POPOVER, scale),
                expected,
                "scale {scale}"
            );
        }
    }

    #[test]
    fn secondary_monitor_left_of_and_above_the_primary() {
        let monitor = rect(-2560, -360, 2560, 1440);
        let work_area = rect(-2560, -360, 2560, 1368);
        let icon = rect(-800, 1008, 36, 72);

        assert_eq!(
            popover_origin(icon, monitor, work_area, POPOVER, 1.5),
            point(-1022, 408)
        );
    }

    #[test]
    fn overflow_flyout_icon_opens_above_it_on_screen() {
        let work_area = rect(0, 0, 1920, 1032);
        let icon = rect(1880, 980, 24, 24);

        assert_eq!(
            popover_origin(icon, MONITOR, work_area, POPOVER, 1.0),
            point(1600, 580)
        );
    }

    #[test]
    fn overflow_flyout_with_the_taskbar_on_the_left_opens_beside_it() {
        let work_area = rect(48, 0, 1872, 1080);
        let icon = rect(60, 1040, 24, 24);

        assert_eq!(
            popover_origin(icon, MONITOR, work_area, POPOVER, 1.0),
            point(84, 680)
        );
    }

    #[test]
    fn auto_hidden_taskbar_uses_the_nearest_screen_edge() {
        let icon = rect(1700, 1040, 24, 40);

        assert_eq!(
            popover_origin(icon, MONITOR, MONITOR, POPOVER, 1.0),
            point(1552, 640)
        );
    }

    #[test]
    fn macos_menu_bar_opens_below_the_icon_centred() {
        let screen = rect(0, 0, 2880, 1800);
        let work_area = rect(0, 74, 2880, 1726);
        let icon = rect(2000, 0, 44, 74);

        assert_eq!(
            popover_origin(icon, screen, work_area, POPOVER, 2.0),
            point(1702, 74)
        );
    }

    #[test]
    fn macos_menu_bar_clamps_at_the_right_edge() {
        let screen = rect(0, 0, 2880, 1800);
        let work_area = rect(0, 74, 2880, 1726);
        let icon = rect(2800, 0, 44, 74);

        assert_eq!(
            popover_origin(icon, screen, work_area, POPOVER, 2.0),
            point(2240, 74)
        );
    }

    #[test]
    fn macos_menu_bar_wins_over_a_larger_dock() {
        let screen = rect(0, 0, 2880, 1800);
        let work_area = rect(0, 74, 2880, 1526);
        let icon = rect(2000, 0, 44, 74);

        assert_eq!(
            popover_origin(icon, screen, work_area, POPOVER, 2.0),
            point(1702, 74)
        );
    }
}
