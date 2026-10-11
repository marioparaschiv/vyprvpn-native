use tray_icon::{BadIcon, Icon};

/// tray-icon takes a single image, which Windows scales to the tray's 16, 20, 24 or 32 px.
/// The largest size only ever downscales, which keeps the anti-aliased edges smooth.
const SIZE: u32 = 32;
const BYTES: usize = (SIZE * SIZE * 4) as usize;

/// Subsamples per pixel along each axis, for coverage-based anti-aliasing.
const SUPERSAMPLE: u32 = 8;
/// Lengths are in units of 1/(2 * SUPERSAMPLE) px, so subsample centres land on integers.
const UNIT: i32 = 2 * SUPERSAMPLE as i32;
/// Leaves a pixel of margin so the anti-aliased rim isn't clipped.
const OUTER_RADIUS: i32 = 15 * UNIT;
/// 3.5 px, about 23% of the radius.
const BORDER_WIDTH: i32 = 7 * UNIT / 2;
const INNER_RADIUS: i32 = OUTER_RADIUS - BORDER_WIDTH;
/// The border's brightness relative to the fill.
const BORDER_SHADE_PERCENT: u32 = 72;

/// The border's width as a share of the dot's radius.
pub const BORDER_SHARE: f32 = BORDER_WIDTH as f32 / OUTER_RADIUS as f32;

/// The connected dot's fill.
pub const CONNECTED: [u8; 3] = [0x8E, 0xE0, 0xA1];

const GREY: [u8; BYTES] = dot([0x9E, 0x9E, 0x9E]);
const BLUE: [u8; BYTES] = dot([0x4C, 0xB1, 0xF5]);
const GREEN: [u8; BYTES] = dot(CONNECTED);

/// Which tray icon shows: grey disconnected, blue busy, green connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    Disconnected,
    Busy,
    Connected,
}

pub fn icon(kind: IconKind) -> Result<Icon, BadIcon> {
    let rgba = match kind {
        IconKind::Disconnected => &GREY,
        IconKind::Busy => &BLUE,
        IconKind::Connected => &GREEN,
    };

    Icon::from_rgba(rgba.to_vec(), SIZE, SIZE)
}

pub const fn darker([r, g, b]: [u8; 3]) -> [u8; 3] {
    const fn shade(channel: u8) -> u8 {
        ((channel as u32 * BORDER_SHADE_PERCENT + 50) / 100) as u8
    }

    [shade(r), shade(g), shade(b)]
}

/// A dot in `fill` ringed by a darker border, in RGBA, computed at compile time so the app ships
/// no image assets.
const fn dot(fill: [u8; 3]) -> [u8; BYTES] {
    let border = darker(fill);
    let samples = SUPERSAMPLE * SUPERSAMPLE;
    let mut rgba = [0; BYTES];
    let mut pixel = 0;
    while pixel < SIZE * SIZE {
        let (in_dot, in_fill) = coverage(pixel % SIZE, pixel / SIZE);
        if in_dot > 0 {
            let in_border = in_dot - in_fill;
            let at = pixel as usize * 4;
            let mut channel = 0;
            while channel < 3 {
                let sum = in_fill * fill[channel] as u32 + in_border * border[channel] as u32;
                rgba[at + channel] = ((sum + in_dot / 2) / in_dot) as u8;
                channel += 1;
            }
            rgba[at + 3] = ((in_dot * 0xFF + samples / 2) / samples) as u8;
        }
        pixel += 1;
    }

    rgba
}

/// How many of the pixel's subsamples fall inside the dot, and how many of those inside the fill.
const fn coverage(x: u32, y: u32) -> (u32, u32) {
    let mut in_dot = 0;
    let mut in_fill = 0;
    let mut sample = 0;
    while sample < SUPERSAMPLE * SUPERSAMPLE {
        let dx = offset(x, sample % SUPERSAMPLE);
        let dy = offset(y, sample / SUPERSAMPLE);
        let distance_squared = dx * dx + dy * dy;
        if distance_squared <= OUTER_RADIUS * OUTER_RADIUS {
            in_dot += 1;
        }
        if distance_squared <= INNER_RADIUS * INNER_RADIUS {
            in_fill += 1;
        }
        sample += 1;
    }

    (in_dot, in_fill)
}

/// The subsample's centre relative to the icon's centre, in `UNIT`s.
const fn offset(pixel: u32, subsample: u32) -> i32 {
    (2 * (pixel * SUPERSAMPLE + subsample) + 1) as i32 - (SIZE * SUPERSAMPLE) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8; BYTES], x: u32, y: u32) -> [u8; 4] {
        let at = ((y * SIZE + x) * 4) as usize;
        [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
    }

    #[test]
    fn border_is_the_fill_darkened_by_28_percent() {
        assert_eq!(darker([0xF2, 0xA9, 0x00]), [0xAE, 0x7A, 0x00]);
        assert_eq!(darker([0x2E, 0xB8, 0x4F]), [0x21, 0x84, 0x39]);
        assert_eq!(darker([0xFF, 0xFF, 0xFF]), [0xB8, 0xB8, 0xB8]);
    }

    #[test]
    fn centre_is_the_opaque_fill() {
        assert_eq!(pixel(&BLUE, SIZE / 2, SIZE / 2), [0x4C, 0xB1, 0xF5, 0xFF]);
        assert_eq!(
            pixel(&GREEN, SIZE / 2 - 1, SIZE / 2 - 1),
            [0x8E, 0xE0, 0xA1, 0xFF]
        );
    }

    #[test]
    fn ring_band_is_the_opaque_border() {
        // Row 3 spans 12 to 13 px from the centre, between the fill's 11.5 and the dot's 15.
        assert_eq!(pixel(&GREEN, SIZE / 2, 3), [0x66, 0xA1, 0x74, 0xFF]);
        assert_eq!(pixel(&GREY, 3, SIZE / 2), [0x72, 0x72, 0x72, 0xFF]);
    }

    #[test]
    fn corners_are_fully_transparent() {
        for (x, y) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
            assert_eq!(pixel(&GREEN, x, y)[3], 0, "corner ({x}, {y})");
        }
    }

    #[test]
    fn edges_are_anti_aliased() {
        // Straddles the dot's rim on the diagonal, 14.1 to 15.6 px from the centre.
        let rim = pixel(&GREEN, 5, 5);
        assert!(0 < rim[3] && rim[3] < 0xFF, "rim alpha {}", rim[3]);
        assert_eq!(rim[..3], [0x66, 0xA1, 0x74]);

        // Straddles the fill's edge at 11.5 px: opaque, between fill and border.
        let seam = pixel(&GREEN, SIZE / 2, 4);
        assert_eq!(seam[3], 0xFF);
        assert!(0xA1 < seam[1] && seam[1] < 0xE0, "seam green {}", seam[1]);
    }
}
