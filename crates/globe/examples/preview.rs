//! Writes the globe as PNGs for eyeballing: `cargo run -p globe --example preview -- <dir> [dot]`.
//! Light and dark show it faint over a popover-like fill; ink shows the bare dots.

use std::num::NonZeroU32;
use std::path::PathBuf;

use image::{Rgb, RgbImage};

const SIZE: u32 = 400;
const OPACITY: f32 = 0.14;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().ok_or("usage: preview <dir> [dot]")?);
    let dot = NonZeroU32::new(args.next().map_or(Ok(1), |dot| dot.parse())?)
        .ok_or("the dot size must be at least 1")?;

    let canvas = globe::Canvas {
        width: SIZE,
        height: SIZE,
        centre: (SIZE as f32 / 2.0, SIZE as f32 / 2.0),
        radius: SIZE as f32 / 2.0 - 4.0,
        dot,
    };
    let dots = canvas.dots();

    std::fs::create_dir_all(&dir)?;
    for (name, background, ink, opacity) in [
        ("light", [255u8, 255, 255], [10u8, 10, 10], OPACITY),
        ("dark", [10, 10, 10], [250, 250, 250], OPACITY),
        ("ink", [255, 255, 255], [0, 0, 0], 1.0),
    ] {
        let dotted = Rgb(std::array::from_fn(|channel| {
            let (background, ink) = (f32::from(background[channel]), f32::from(ink[channel]));
            (background + (ink - background) * opacity).round() as u8
        }));
        let picture = RgbImage::from_fn(SIZE, SIZE, |x, y| {
            if dots[(y * SIZE + x) as usize] {
                dotted
            } else {
                Rgb(background)
            }
        });
        let path = dir.join(format!("globe-{name}-{dot}.png"));
        picture.save(&path)?;
        println!("{}", path.display());
    }

    Ok(())
}
