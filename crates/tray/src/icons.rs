use tray_icon::{BadIcon, Icon};

const SIZE: u32 = 32;
const BYTES: usize = (SIZE * SIZE * 4) as usize;

const GREY: [u8; BYTES] = circle([0x9E, 0x9E, 0x9E]);
const AMBER: [u8; BYTES] = circle([0xF2, 0xA9, 0x00]);
const GREEN: [u8; BYTES] = circle([0x2E, 0xB8, 0x4F]);

/// Which tray icon shows: grey disconnected, amber busy, green connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Disconnected,
    Busy,
    Connected,
}

pub fn icon(kind: IconKind) -> Result<Icon, BadIcon> {
    let rgba = match kind {
        IconKind::Disconnected => &GREY,
        IconKind::Busy => &AMBER,
        IconKind::Connected => &GREEN,
    };

    Icon::from_rgba(rgba.to_vec(), SIZE, SIZE)
}

/// A filled circle in RGBA, computed at compile time so the app ships no image assets.
const fn circle([r, g, b]: [u8; 3]) -> [u8; BYTES] {
    let mut rgba = [0; BYTES];
    let mut pixel = 0;
    while pixel < SIZE * SIZE {
        // Doubled coordinates of the pixel's centre relative to the icon's centre, to stay in
        // integers.
        let dx = (2 * (pixel % SIZE) + 1) as i32 - SIZE as i32;
        let dy = (2 * (pixel / SIZE) + 1) as i32 - SIZE as i32;
        if dx * dx + dy * dy <= (SIZE * SIZE) as i32 {
            let at = pixel as usize * 4;
            rgba[at] = r;
            rgba[at + 1] = g;
            rgba[at + 2] = b;
            rgba[at + 3] = 0xFF;
        }
        pixel += 1;
    }

    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(rgba: &[u8; BYTES], x: u32, y: u32) -> u8 {
        rgba[((y * SIZE + x) * 4 + 3) as usize]
    }

    #[test]
    fn draws_an_opaque_circle_on_a_transparent_square() {
        assert_eq!(alpha(&GREEN, SIZE / 2, SIZE / 2), 0xFF);
        assert_eq!(alpha(&GREEN, SIZE / 2, 0), 0xFF);
        assert_eq!(alpha(&GREEN, 0, 0), 0);
        assert_eq!(alpha(&GREEN, SIZE - 1, SIZE - 1), 0);

        let centre = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
        assert_eq!(AMBER[centre..centre + 4], [0xF2, 0xA9, 0x00, 0xFF]);
    }
}
