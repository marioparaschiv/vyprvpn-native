//! The dithered globe behind the main page's status, drawn once for each text colour and display
//! scale it is shown at.

use std::num::NonZeroU32;
use std::sync::Arc;

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use image::{Frame, Rgba as Pixel, RgbaImage};

use super::CARD;

/// The globe's centre height and radius on the main page, in logical pixels. It is centred
/// horizontally, cropped by the card's top and sides, and stops above the Connect button.
const CENTRE_Y: f32 = 125.;
const RADIUS: f32 = 185.;
const HEIGHT: f32 = 310.;

/// Faint enough that text over the dots stays readable.
const OPACITY: f32 = 0.14;

const TWO: NonZeroU32 = NonZeroU32::new(2).unwrap();

#[derive(Default)]
pub struct Globe {
    drawn: Option<Drawn>,
}

struct Drawn {
    scale: f32,
    color: Rgba,
    image: Arc<RenderImage>,
}

impl Globe {
    pub fn render(&mut self, window: &mut Window, cx: &App) -> Img {
        let scale = window.scale_factor();
        let color = cx.theme().popover_foreground.opacity(OPACITY).to_rgb();
        let drawn = match self.drawn.take() {
            Some(drawn) if drawn.scale == scale && drawn.color == color => drawn,
            stale => {
                if let Some(stale) = stale
                    && let Err(error) = window.drop_image(stale.image)
                {
                    log::warn!("freeing the previous globe failed: {error}");
                }
                Drawn {
                    scale,
                    color,
                    image: draw(scale, color),
                }
            }
        };
        let size = drawn.image.size(0);
        let image = drawn.image.clone();
        self.drawn = Some(drawn);

        img(image)
            .absolute()
            .top_0()
            .left_0()
            .w(px(size.width.0 as f32 / scale))
            .h(px(size.height.0 as f32 / scale))
    }
}

/// Each dot covers whole device pixels: one on standard displays, two from 150% scale up.
fn draw(scale: f32, color: Rgba) -> Arc<RenderImage> {
    let width = CARD.width.as_f32() * scale;
    let canvas = globe::Canvas {
        width: width.round() as u32,
        height: (HEIGHT * scale).round() as u32,
        centre: (width / 2., CENTRE_Y * scale),
        radius: RADIUS * scale,
        dot: if scale >= 1.5 { TWO } else { NonZeroU32::MIN },
    };
    let dots = canvas.dots();

    let channel = |value: f32| (value * 255.).round() as u8;
    // GPUI's images are BGRA with straight alpha.
    let dot = Pixel([
        channel(color.b),
        channel(color.g),
        channel(color.r),
        channel(color.a),
    ]);
    let buffer = RgbaImage::from_fn(canvas.width, canvas.height, |x, y| {
        if dots[(y * canvas.width + x) as usize] {
            dot
        } else {
            Pixel([0; 4])
        }
    });

    Arc::new(RenderImage::new([Frame::new(buffer)]))
}
