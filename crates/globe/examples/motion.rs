//! Writes frames of the globe's motion as PNGs and times drawing them into an image buffer:
//! `cargo run --release -p globe --example motion -- <dir>`. The canvas is the popover's at 150%
//! scale, in ink on white, with the marker as a plain ringed disc.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use globe::motion::{Motion, Scene, Target};
use globe::{Canvas, places};
use image::{Rgb, RgbImage};

const SCALE: f32 = 1.5;
const WIDTH: f32 = 340.;
const HEIGHT: f32 = 310.;
const CENTRE_Y: f32 = 125.;
const RADIUS: f32 = 185.;
const MARKER: f32 = 12.;

const FILL: Rgb<u8> = Rgb([0x8E, 0xE0, 0xA1]);
const RING: Rgb<u8> = Rgb([0x66, 0xA1, 0x74]);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(std::env::args().nth(1).ok_or("usage: motion <dir>")?);
    std::fs::create_dir_all(&dir)?;

    let start = Instant::now();
    let spin = Motion::new(Target::Spin, start);
    for millis in [0, 300, 600, 1000, 1500, 2000] {
        let at = start + Duration::from_millis(millis);
        save(&dir, &format!("spin-{millis:04}ms"), spin.scene(at))?;
    }

    for (name, spoke_id, country_code) in [("germany", "de1", "GER"), ("perth", "au3", "AUS")] {
        let place = places::place(spoke_id, Some(country_code)).ok_or("unknown place")?;
        let connected = start + Duration::from_millis(2300);
        let settle = spin.retarget(Target::Place(place), connected);
        for millis in [300, 600] {
            let at = connected + Duration::from_millis(millis);
            save(&dir, &format!("{name}-{millis:04}ms"), settle.scene(at))?;
        }
        save(
            &dir,
            &format!("{name}-final"),
            settle.scene(connected + Duration::from_secs(5)),
        )?;
    }

    time(spin.scene(start + Duration::from_secs(2)), "spinning");
    let zoomed = Motion::new(
        Target::Place(places::place("au3", None).ok_or("no Perth")?),
        start,
    );
    time(zoomed.scene(start), "zoomed in");

    Ok(())
}

fn canvas(scene: Scene) -> Canvas {
    let width = WIDTH * SCALE;

    Canvas {
        width: width.round() as u32,
        height: (HEIGHT * SCALE).round() as u32,
        centre: (width / 2., CENTRE_Y * SCALE),
        radius: RADIUS * scene.pose.zoom * SCALE,
        facing: scene.pose.facing,
        dot: NonZeroU32::new(2).expect("two is not zero"),
    }
}

fn save(dir: &Path, name: &str, scene: Scene) -> Result<(), Box<dyn std::error::Error>> {
    let canvas = canvas(scene);
    let dots = canvas.dots();
    let mut picture = RgbImage::from_fn(canvas.width, canvas.height, |x, y| {
        if dots[(y * canvas.width + x) as usize] {
            Rgb([0, 0, 0])
        } else {
            Rgb([255, 255, 255])
        }
    });

    let marker = scene
        .marker
        .and_then(|marker| Some((canvas.project(marker.at)?, marker.scale)));
    if let Some(((mx, my), scale)) = marker {
        let outer = MARKER * SCALE * scale / 2.;
        let inner = outer * (1. - 3.5 / 15.);
        for (x, y, pixel) in picture.enumerate_pixels_mut() {
            let distance = (x as f32 + 0.5 - mx).hypot(y as f32 + 0.5 - my);
            if distance <= inner {
                *pixel = FILL;
            } else if distance <= outer {
                *pixel = RING;
            }
        }
    }

    let path = dir.join(format!("{name}.png"));
    picture.save(&path)?;
    println!("{}", path.display());

    Ok(())
}

fn time(scene: Scene, name: &str) {
    const RUNS: u32 = 50;
    let canvas = canvas(scene);

    let start = Instant::now();
    for _ in 0..RUNS {
        let dots = canvas.dots();
        let buffer = image::RgbaImage::from_fn(canvas.width, canvas.height, |x, y| {
            image::Rgba([
                0,
                0,
                0,
                u8::from(dots[(y * canvas.width + x) as usize]) * 36,
            ])
        });
        std::hint::black_box(buffer);
    }
    println!(
        "{name}: {:.2} ms a frame at {}x{}",
        start.elapsed().as_secs_f64() * 1000. / f64::from(RUNS),
        canvas.width,
        canvas.height
    );
}
