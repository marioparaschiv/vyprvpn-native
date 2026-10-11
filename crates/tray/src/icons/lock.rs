use tray_icon::Icon;

use super::{CONNECTED, IconError, IconKind};

/// tray-icon takes a single image, which Windows scales to the tray's 16, 20, 24 or 32 px.
/// The largest size only ever downscales, which keeps the anti-aliased edges smooth.
const SIZE: u32 = 32;
const BYTES: usize = (SIZE * SIZE * 4) as usize;

/// Subsamples per pixel along each axis, for coverage-based anti-aliasing.
const SUPERSAMPLE: u32 = 8;
/// Lucide draws on a 24-unit viewBox.
const PX_PER_UNIT: f32 = SIZE as f32 / 24.;
/// Half of Lucide's stroke-width of 2.
const HALF_STROKE: f32 = 1.;

const WHITE: [u8; BYTES] = lock(Lock::Open, [0xFF, 0xFF, 0xFF]);
const BLUE: [u8; BYTES] = lock(Lock::Open, [0x4C, 0xB1, 0xF5]);
const GREEN: [u8; BYTES] = lock(Lock::Closed, CONNECTED);

/// A white open lock disconnected, blue open lock busy, green closed lock connected.
pub fn icon(kind: IconKind) -> Result<Icon, IconError> {
    let rgba = match kind {
        IconKind::Disconnected => &WHITE,
        IconKind::Busy => &BLUE,
        IconKind::Connected => &GREEN,
    };

    Ok(Icon::from_rgba(rgba.to_vec(), SIZE, SIZE)?)
}

#[derive(Clone, Copy)]
enum Lock {
    /// Lucide's `lock-open`.
    Open,
    /// Lucide's `lock`.
    Closed,
}

/// Lucide's lock stroked in `colour`, in RGBA, computed at compile time so the app ships no image
/// assets.
const fn lock(shape: Lock, colour: [u8; 3]) -> [u8; BYTES] {
    let samples = SUPERSAMPLE * SUPERSAMPLE;
    let mut rgba = [0; BYTES];
    let mut pixel = 0;
    while pixel < SIZE * SIZE {
        let covered = coverage(shape, pixel % SIZE, pixel / SIZE);
        if covered > 0 {
            let at = pixel as usize * 4;
            rgba[at] = colour[0];
            rgba[at + 1] = colour[1];
            rgba[at + 2] = colour[2];
            rgba[at + 3] = ((covered * 0xFF + samples / 2) / samples) as u8;
        }
        pixel += 1;
    }

    rgba
}

/// How many of the pixel's subsamples fall on the lock's stroke.
const fn coverage(shape: Lock, x: u32, y: u32) -> u32 {
    let mut covered = 0;
    let mut sample = 0;
    while sample < SUPERSAMPLE * SUPERSAMPLE {
        let ux = to_units(x, sample % SUPERSAMPLE);
        let uy = to_units(y, sample / SUPERSAMPLE);
        if on_body(ux, uy) || on_shackle(shape, ux, uy) {
            covered += 1;
        }
        sample += 1;
    }

    covered
}

/// The subsample's centre in Lucide's viewBox units.
const fn to_units(pixel: u32, subsample: u32) -> f32 {
    (pixel as f32 + (subsample as f32 + 0.5) / SUPERSAMPLE as f32) / PX_PER_UNIT
}

/// `<rect width="18" height="11" x="3" y="11" rx="2" ry="2" />`
const fn on_body(x: f32, y: f32) -> bool {
    let h = HALF_STROKE;

    in_rounded_rect(x, y, [3. - h, 11. - h, 21. + h, 22. + h], 2. + h)
        && !in_rounded_rect(x, y, [3. + h, 11. + h, 21. - h, 22. - h], 2. - h)
}

/// `lock`: `M7 11V7a5 5 0 0 1 10 0v4`, `lock-open`: `M7 11V7a5 5 0 0 1 9.9-1`.
/// The legs' bottom caps hide under the body's stroke, so only the open end needs one.
const fn on_shackle(shape: Lock, x: f32, y: f32) -> bool {
    let (dx, dy) = (x - 12., y - 7.);
    let distance_squared = dx * dx + dy * dy;
    let on_ring = dy <= 0.
        && (5. - HALF_STROKE) * (5. - HALF_STROKE) <= distance_squared
        && distance_squared <= (5. + HALF_STROKE) * (5. + HALF_STROKE);
    let on_left_leg = on_leg(7., x, y);

    match shape {
        Lock::Closed => on_ring || on_left_leg || on_leg(17., x, y),
        Lock::Open => {
            // The arc stops at (16.9, 6), so the ring is cut along the ray from its centre there.
            let before_end = 4.9 * dy + dx <= 0.;
            let (ex, ey) = (x - 16.9, y - 6.);
            let on_end_cap = ex * ex + ey * ey <= HALF_STROKE * HALF_STROKE;

            (on_ring && before_end) || on_end_cap || on_left_leg
        }
    }
}

/// A vertical leg from the ring's ends at y = 7 down into the body at y = 11.
const fn on_leg(centre_x: f32, x: f32, y: f32) -> bool {
    (x - centre_x).abs() <= HALF_STROKE && 7. <= y && y <= 11.
}

const fn in_rounded_rect(
    x: f32,
    y: f32,
    [left, top, right, bottom]: [f32; 4],
    radius: f32,
) -> bool {
    let dx = x - x.clamp(left + radius, right - radius);
    let dy = y - y.clamp(top + radius, bottom - radius);

    dx * dx + dy * dy <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(rgba: &[u8; BYTES], x: u32, y: u32) -> [u8; 4] {
        let at = ((y * SIZE + x) * 4) as usize;
        [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
    }

    #[test]
    fn body_stroke_is_the_opaque_colour() {
        // Spans 3 to 3.75 units across, inside the left edge's 2 to 4 stroke.
        assert_eq!(pixel(&WHITE, 4, 22), [0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(pixel(&GREEN, 4, 22), [0x8E, 0xE0, 0xA1, 0xFF]);
    }

    #[test]
    fn body_is_hollow() {
        assert_eq!(pixel(&GREEN, SIZE / 2, 22)[3], 0);
    }

    #[test]
    fn only_the_closed_lock_has_a_right_leg() {
        // Spans 16.5 to 17.25 units across and 9 to 9.75 down, on the right leg.
        assert_eq!(pixel(&GREEN, 22, 12)[3], 0xFF);
        assert_eq!(pixel(&WHITE, 22, 12)[3], 0);
        assert_eq!(pixel(&BLUE, 22, 12)[3], 0);
    }

    #[test]
    fn both_locks_have_the_arc_top() {
        // The arc's top is at 2 units, 1 to 3 with the stroke: 1.33 to 4 px.
        assert_eq!(pixel(&WHITE, SIZE / 2, 2)[3], 0xFF);
        assert_eq!(pixel(&GREEN, SIZE / 2, 2)[3], 0xFF);
    }

    #[test]
    fn corners_are_fully_transparent() {
        for (x, y) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
            assert_eq!(pixel(&GREEN, x, y)[3], 0, "corner ({x}, {y})");
        }
    }

    #[test]
    fn edges_are_anti_aliased() {
        // Spans 1.5 to 2.25 units across, straddling the body's outer edge at 2.
        let rim = pixel(&GREEN, 2, 22);
        assert!(0 < rim[3] && rim[3] < 0xFF, "rim alpha {}", rim[3]);
    }
}
