//! A retro picture of the Earth: an orthographic globe turned to any point, lit from the upper left
//! and drawn as 1-bit dots with an ordered (Bayer) dither.

pub mod motion;
pub mod places;

use std::num::NonZeroU32;

/// Land mask columns: one per half degree of longitude, from 180°W eastwards.
pub const MASK_COLUMNS: usize = 720;
/// Land mask rows: one per half degree of latitude, from 90°N southwards.
pub const MASK_ROWS: usize = 360;

/// Natural Earth's 1:110m land polygons (public domain, <https://www.naturalearthdata.com>),
/// rasterised by `cargo xtask land-mask`: one bit per cell, rows north to south, the most
/// significant bit first.
static LAND: &[u8; MASK_COLUMNS * MASK_ROWS / 8] = include_bytes!("land.bin");

/// A point on the Earth, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinates {
    pub lon: f32,
    pub lat: f32,
}

/// The resting view: the Atlantic off West Africa faces the viewer, which tilts Europe into the
/// upper half.
pub const HOME: Coordinates = Coordinates {
    lon: -5.0,
    lat: 20.0,
};

/// Where the light comes from, in view space (x right, y up, z toward the viewer).
const LIGHT: [f32; 3] = [-0.6, 0.6, 1.0];

/// How much of the surface is dotted where fully lit, from 0 (none) to 1 (all). Shading scales both
/// alike, so land stays denser than sea even on the shadowed side.
const LAND_COVER: f32 = 0.9;
const SEA_COVER: f32 = 0.26;
/// The share of the light that reaches the side turned away from it.
const AMBIENT: f32 = 0.35;
const RIM: f32 = 0.5;

const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// Whether the land mask has land at a longitude and latitude in degrees.
pub fn is_land(lon: f32, lat: f32) -> bool {
    let column = ((lon + 180.0).rem_euclid(360.0) / 360.0 * MASK_COLUMNS as f32) as usize;
    let row = ((90.0 - lat) / 180.0 * MASK_ROWS as f32).clamp(0.0, MASK_ROWS as f32 - 1.0) as usize;
    let bit = row * MASK_COLUMNS + column.min(MASK_COLUMNS - 1);

    LAND[bit / 8] & (0x80 >> (bit % 8)) != 0
}

/// The dither threshold for a dot cell, strictly between 0 and 1.
fn threshold(cell_x: u32, cell_y: u32) -> f32 {
    let rank = BAYER[cell_y as usize % 8][cell_x as usize % 8];

    (f32::from(rank) + 0.5) / 64.0
}

/// An image of the globe, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    /// The globe's centre, which may lie anywhere, so the globe can be cropped.
    pub centre: (f32, f32),
    pub radius: f32,
    /// The point drawn at the globe's centre.
    pub facing: Coordinates,
    /// The side of each square dot.
    pub dot: NonZeroU32,
}

impl Canvas {
    /// Where a point on the Earth lands on the canvas, or `None` on the far side.
    pub fn project(&self, point: Coordinates) -> Option<(f32, f32)> {
        let (x, y, z) = Turn::new(self.facing).to_view(point);
        if z < 0.0 {
            return None;
        }

        Some((
            self.centre.0 + x * self.radius,
            self.centre.1 - y * self.radius,
        ))
    }

    /// Row-major, one entry per pixel: whether it is part of a dot.
    pub fn dots(&self) -> Vec<bool> {
        let dot = self.dot.get();
        let mut dots = vec![false; self.width as usize * self.height as usize];
        let length = LIGHT.iter().map(|axis| axis * axis).sum::<f32>().sqrt();
        let light = LIGHT.map(|axis| axis / length);
        let turn = Turn::new(self.facing);

        for cell_y in 0..self.height.div_ceil(dot) {
            for cell_x in 0..self.width.div_ceil(dot) {
                let x = (cell_x * dot) as f32 + dot as f32 / 2.0;
                let y = (cell_y * dot) as f32 + dot as f32 / 2.0;
                let shown = self
                    .tone(x, y, light, &turn)
                    .is_some_and(|tone| tone > threshold(cell_x, cell_y));
                if !shown {
                    continue;
                }

                let left = (cell_x * dot) as usize;
                let right = ((cell_x + 1) * dot).min(self.width) as usize;
                for py in cell_y * dot..((cell_y + 1) * dot).min(self.height) {
                    let row = py as usize * self.width as usize;
                    dots[row + left..row + right].fill(true);
                }
            }
        }

        dots
    }

    /// How densely the globe is dotted at a pixel, or `None` off the globe.
    fn tone(&self, x: f32, y: f32, light: [f32; 3], turn: &Turn) -> Option<f32> {
        let nx = (x - self.centre.0) / self.radius;
        let ny = (self.centre.1 - y) / self.radius;
        let distance = nx * nx + ny * ny;
        if distance > 1.0 {
            return None;
        }
        let nz = (1.0 - distance).sqrt();

        let lambert = (nx * light[0] + ny * light[1] + nz * light[2]).max(0.0);
        let point = turn.to_earth(nx, ny, nz);
        let cover = if is_land(point.lon, point.lat) {
            LAND_COVER
        } else {
            SEA_COVER
        };
        let surface = cover * (AMBIENT + (1.0 - AMBIENT) * lambert);
        let on_rim = distance.sqrt() > 1.0 - self.dot.get() as f32 / self.radius;

        Some(if on_rim { surface.max(RIM) } else { surface })
    }
}

/// The rotation that brings a point on the Earth to face the viewer.
struct Turn {
    lon: f32,
    sin_tilt: f32,
    cos_tilt: f32,
}

impl Turn {
    fn new(facing: Coordinates) -> Self {
        let (sin_tilt, cos_tilt) = facing.lat.to_radians().sin_cos();

        Self {
            lon: facing.lon,
            sin_tilt,
            cos_tilt,
        }
    }

    /// A point on the Earth in view space (x right, y up, z toward the viewer), on the unit sphere.
    fn to_view(&self, point: Coordinates) -> (f32, f32, f32) {
        let (sin_lat, cos_lat) = point.lat.to_radians().sin_cos();
        let (sin_lon, cos_lon) = (point.lon - self.lon).to_radians().sin_cos();

        (
            cos_lat * sin_lon,
            self.cos_tilt * sin_lat - self.sin_tilt * cos_lat * cos_lon,
            self.sin_tilt * sin_lat + self.cos_tilt * cos_lat * cos_lon,
        )
    }

    /// The point on the Earth under a point on the unit sphere in view space.
    fn to_earth(&self, x: f32, y: f32, z: f32) -> Coordinates {
        let lat = (y * self.cos_tilt + z * self.sin_tilt)
            .clamp(-1.0, 1.0)
            .asin();
        let lon = x.atan2(z * self.cos_tilt - y * self.sin_tilt);

        Coordinates {
            lon: self.lon + lon.to_degrees(),
            lat: lat.to_degrees(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: Canvas = Canvas {
        width: 400,
        height: 300,
        centre: (200.0, 160.0),
        radius: 180.0,
        facing: HOME,
        dot: NonZeroU32::MIN,
    };

    fn at(lon: f32, lat: f32) -> Coordinates {
        Coordinates { lon, lat }
    }

    fn assert_near((x, y): (f32, f32), (expected_x, expected_y): (f32, f32)) {
        assert!(
            (x - expected_x).abs() < 0.01 && (y - expected_y).abs() < 0.01,
            "({x}, {y}) is not ({expected_x}, {expected_y})"
        );
    }

    #[test]
    fn projects_onto_the_facing_hemisphere() {
        assert_near(CANVAS.project(HOME).unwrap(), CANVAS.centre);
        assert_near(
            CANVAS.project(at(HOME.lon + 60.0, 0.0)).unwrap(),
            (
                200.0 + 180.0 * 60f32.to_radians().sin(),
                160.0 + 180.0 * 20f32.to_radians().sin() * 60f32.to_radians().cos(),
            ),
        );
        assert_near(
            CANVAS.project(at(0.0, 90.0)).unwrap(),
            (200.0, 160.0 - 180.0 * 20f32.to_radians().cos()),
        );
    }

    #[test]
    fn culls_the_far_hemisphere() {
        assert_eq!(CANVAS.project(at(HOME.lon + 180.0, 0.0)), None);
        assert_eq!(CANVAS.project(at(HOME.lon + 180.0, -HOME.lat)), None);
        assert_eq!(CANVAS.project(at(0.0, -90.0)), None);
    }

    #[test]
    fn a_turned_and_zoomed_globe_puts_the_facing_point_at_its_centre() {
        let perth = at(115.86, -31.95);
        let zoomed = Canvas {
            facing: perth,
            radius: CANVAS.radius * 1.8,
            ..CANVAS
        };

        assert_near(zoomed.project(perth).unwrap(), zoomed.centre);
        // A degree east of Perth lies right of the centre, 1.8 times as far as unzoomed.
        let east = at(perth.lon + 1.0, perth.lat);
        let unzoomed = Canvas {
            facing: perth,
            ..CANVAS
        };
        let zoomed_offset = zoomed.project(east).unwrap().0 - zoomed.centre.0;
        let unzoomed_offset = unzoomed.project(east).unwrap().0 - unzoomed.centre.0;
        assert!(unzoomed_offset > 0.0);
        assert!((zoomed_offset - unzoomed_offset * 1.8).abs() < 0.01);
        assert_eq!(zoomed.project(at(perth.lon + 180.0, -perth.lat)), None);
    }

    #[test]
    fn unprojects_what_it_projects() {
        let turn = Turn::new(CANVAS.facing);
        for (lon, lat) in [(2.35, 48.85), (-43.2, -22.9), (31.2, 30.0), (-74.0, 40.7)] {
            let (x, y) = CANVAS.project(at(lon, lat)).unwrap();
            let nx = (x - CANVAS.centre.0) / CANVAS.radius;
            let ny = (CANVAS.centre.1 - y) / CANVAS.radius;
            let nz = (1.0 - nx * nx - ny * ny).sqrt();
            let point = turn.to_earth(nx, ny, nz);

            assert_near((point.lon, point.lat), (lon, lat));
        }
    }

    #[test]
    fn bayer_matrix_spreads_every_threshold_once() {
        let mut ranks: Vec<u8> = BAYER.iter().flatten().copied().collect();
        ranks.sort_unstable();
        assert_eq!(ranks, (0..64).collect::<Vec<_>>());

        // The first four dots of a filling cell fall in different quadrants of the tile.
        let quadrants: Vec<(usize, usize)> = (0..4)
            .map(|rank| {
                let index = BAYER.iter().flatten().position(|&r| r == rank).unwrap();
                (index / 8 / 4, index % 8 / 4)
            })
            .collect();
        assert_eq!(quadrants, [(0, 0), (1, 1), (0, 1), (1, 0)]);

        assert_eq!(threshold(0, 0), 0.5 / 64.0);
        assert_eq!(threshold(8, 3), threshold(0, 3));
        assert_eq!(threshold(7, 7), 21.5 / 64.0);
    }

    #[test]
    fn knows_land_from_sea() {
        let land = [(10.0, 23.0), (2.35, 48.85), (134.0, -25.0), (-100.0, 40.0)];
        let sea = [(-35.0, 30.0), (-150.0, 0.0), (80.0, -20.0), (-4.0, 44.5)];

        for (lon, lat) in land {
            assert!(is_land(lon, lat), "({lon}, {lat}) should be land");
        }
        for (lon, lat) in sea {
            assert!(!is_land(lon, lat), "({lon}, {lat}) should be sea");
        }
        assert!(
            is_land(180.0, -89.9),
            "Antarctica wraps at the antimeridian"
        );
    }

    fn density(dots: &[bool], canvas: &Canvas, (x, y): (f32, f32)) -> usize {
        let (x, y) = (x as usize, y as usize);

        (y - 8..y + 8)
            .flat_map(|row| (x - 8..x + 8).map(move |column| (row, column)))
            .filter(|&(row, column)| dots[row * canvas.width as usize + column])
            .count()
    }

    #[test]
    fn dots_land_more_densely_than_sea() {
        let dots = CANVAS.dots();
        let sahara = density(&dots, &CANVAS, CANVAS.project(at(10.0, 23.0)).unwrap());
        let atlantic = density(&dots, &CANVAS, CANVAS.project(at(-35.0, 30.0)).unwrap());

        assert!(
            sahara > atlantic * 2,
            "Sahara {sahara}, Atlantic {atlantic}"
        );
    }

    #[test]
    fn dots_nothing_off_the_globe() {
        let dots = CANVAS.dots();
        let off_globe = (0..CANVAS.height).flat_map(|y| (0..CANVAS.width).map(move |x| (x, y)));

        for (x, y) in off_globe {
            let dx = x as f32 + 0.5 - CANVAS.centre.0;
            let dy = y as f32 + 0.5 - CANVAS.centre.1;
            if dx * dx + dy * dy > CANVAS.radius * CANVAS.radius + 1.0 {
                assert!(!dots[(y * CANVAS.width + x) as usize], "dot at ({x}, {y})");
            }
        }
    }

    #[test]
    fn draws_the_same_whole_dots_each_time() {
        let canvas = Canvas {
            dot: NonZeroU32::new(2).unwrap(),
            ..CANVAS
        };
        let dots = canvas.dots();

        assert_eq!(dots, canvas.dots());
        assert!(dots.iter().any(|&dot| dot));
        for y in (0..canvas.height).step_by(2) {
            for x in (0..canvas.width).step_by(2) {
                let at = |dx: u32, dy: u32| dots[((y + dy) * canvas.width + x + dx) as usize];
                let block = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
                assert!(
                    block.iter().all(|&dot| dot == block[0]),
                    "split dot at ({x}, {y})"
                );
            }
        }
    }
}
