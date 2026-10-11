//! The dithered globe behind the main page's status: at rest while disconnected, spinning while
//! connecting, and turned and zoomed onto the server, marked with a ringed dot in the
//! tray's connected colour, once connected. It is redrawn only for the frames in which it moves.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use ::globe::Canvas;
use ::globe::motion::{Marker, Motion, Pose, Target, ZOOM};
use ::globe::places;
use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use image::{Frame, Rgba as Pixel, RgbaImage};
use ipc::Status;

use super::CARD;
use crate::helper_link::{HelperState, LinkState};
use crate::icons;

/// The globe's centre height and radius on the main page, in logical pixels, before zooming. It is
/// centred horizontally, cropped by the card's top and sides, and stops above the Connect button.
const CENTRE_Y: f32 = 125.;
const RADIUS: f32 = 185.;
pub const HEIGHT: f32 = 310.;
/// Where the facing point sits once fully zoomed in: up and right of the status icon, so the
/// marker stays clear of it and of the headline.
const ZOOMED_CENTRE: (f32, f32) = (250., 70.);

/// Faint enough that text over the dots stays readable.
const OPACITY: f32 = 0.14;

const TWO: NonZeroU32 = NonZeroU32::new(2).unwrap();

/// The marker's diameter at full size, in logical pixels.
const MARKER: f32 = 12.;
/// The marker's ring width as a share of its radius.
const MARKER_RING_SHARE: f32 = 0.23;
/// The ring's brightness relative to the fill.
const MARKER_RING_SHADE_PERCENT: u32 = 72;

pub struct Globe {
    motion: Motion,
    drawn: Option<Drawn>,
}

struct Drawn {
    scale: f32,
    color: Rgba,
    pose: Pose,
    image: Arc<RenderImage>,
}

/// Where the globe heads for in a connection state: the server's place once connected, if known.
pub fn target(state: &LinkState) -> Target {
    match &state.helper {
        HelperState::Running(Status::Connecting { .. }) => Target::Spin,
        HelperState::Running(Status::Connected { spoke_id, .. }) => {
            let country_code = state
                .locations
                .find(spoke_id)
                .map(|location| location.country_code.as_str());

            places::place(spoke_id, country_code).map_or(Target::Home, Target::Place)
        }
        _ => Target::Home,
    }
}

impl Globe {
    /// Shows `target` without an animation, other than spinning.
    pub fn new(target: Target, now: Instant, reduce_motion: bool) -> Self {
        Self {
            motion: if reduce_motion {
                Motion::still(target, now)
            } else {
                Motion::new(target, now)
            },
            drawn: None,
        }
    }

    pub fn set_target(&mut self, target: Target, now: Instant, reduce_motion: bool) {
        if target == self.motion.target() {
            return;
        }

        self.motion = if reduce_motion {
            Motion::still(target, now)
        } else {
            self.motion.retarget(target, now)
        };
    }

    pub fn render(&mut self, window: &mut Window, cx: &App) -> Div {
        let scene = self.motion.scene(cx.background_executor().now());
        if scene.moving {
            window.request_animation_frame();
        }

        let scale = window.scale_factor();
        let color = cx.theme().popover_foreground.opacity(OPACITY).to_rgb();
        let drawn = match self.drawn.take() {
            Some(drawn)
                if drawn.scale == scale && drawn.color == color && drawn.pose == scene.pose =>
            {
                drawn
            }
            stale => {
                if let Some(stale) = stale
                    && let Err(error) = window.drop_image(stale.image)
                {
                    log::warn!("freeing the previous globe failed: {error}");
                }
                Drawn {
                    scale,
                    color,
                    pose: scene.pose,
                    image: draw(&canvas(scale, scene.pose), color),
                }
            }
        };
        let size = drawn.image.size(0);
        let image = img(drawn.image.clone())
            .absolute()
            .top_0()
            .left_0()
            .w(px(size.width.0 as f32 / scale))
            .h(px(size.height.0 as f32 / scale));
        self.drawn = Some(drawn);

        div().absolute().size_full().child(image).children(
            scene
                .marker
                .and_then(|marker| render_marker(marker, &canvas(scale, scene.pose), scale)),
        )
    }
}

/// The canvas in device pixels. The globe's centre slides toward [`ZOOMED_CENTRE`] as it zooms. Each dot covers whole device pixels: one on standard displays, two
/// from 150% scale up.
fn canvas(scale: f32, pose: Pose) -> Canvas {
    let width = CARD.width.as_f32();
    let zoomed = (pose.zoom - 1.) / (ZOOM - 1.);
    let centre_x = width / 2. + (ZOOMED_CENTRE.0 - width / 2.) * zoomed;
    let centre_y = CENTRE_Y + (ZOOMED_CENTRE.1 - CENTRE_Y) * zoomed;

    Canvas {
        width: (width * scale).round() as u32,
        height: (HEIGHT * scale).round() as u32,
        centre: (centre_x * scale, centre_y * scale),
        radius: RADIUS * pose.zoom * scale,
        facing: pose.facing,
        dot: if scale >= 1.5 { TWO } else { NonZeroU32::MIN },
    }
}

fn draw(canvas: &Canvas, color: Rgba) -> Arc<RenderImage> {
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

/// A dot in the tray's connected colour, ringed darker, centred on the marked place.
fn render_marker(marker: Marker, canvas: &Canvas, scale: f32) -> Option<Div> {
    let (x, y) = canvas.project(marker.at)?;
    let diameter = MARKER * marker.scale;
    let [fill, ring] = [icons::CONNECTED, darker(icons::CONNECTED)]
        .map(|[r, g, b]| rgb(u32::from_be_bytes([0, r, g, b])));

    Some(
        div()
            .absolute()
            .left(px(x / scale - diameter / 2.))
            .top(px(y / scale - diameter / 2.))
            .size(px(diameter))
            .rounded_full()
            .bg(fill)
            .border(px(diameter / 2. * MARKER_RING_SHARE))
            .border_color(ring),
    )
}

fn darker([r, g, b]: [u8; 3]) -> [u8; 3] {
    let shade = |channel: u8| ((channel as u32 * MARKER_RING_SHADE_PERCENT + 50) / 100) as u8;

    [shade(r), shade(g), shade(b)]
}

#[cfg(test)]
mod tests {
    use ::globe::Coordinates;
    use ipc::Status;

    use std::time::{Duration, Instant};

    use super::{Globe, ZOOMED_CENTRE, canvas, darker, target};
    use crate::helper_link::{HelperState, Locations};
    use crate::popover::model::tests::state;
    use crate::popover::recorded_locations;
    use ::globe::motion::Target;

    fn connected(spoke_id: &str) -> Status {
        Status::Connected {
            spoke_id: spoke_id.into(),
            tunnel_ip: [10, 1, 2, 3].into(),
            connected_at: 1_760_140_800,
        }
    }

    #[test]
    fn ring_is_the_fill_darkened_by_28_percent() {
        assert_eq!(darker([0xF2, 0xA9, 0x00]), [0xAE, 0x7A, 0x00]);
        assert_eq!(darker([0x2E, 0xB8, 0x4F]), [0x21, 0x84, 0x39]);
        assert_eq!(darker([0xFF, 0xFF, 0xFF]), [0xB8, 0xB8, 0xB8]);
    }

    #[test]
    fn the_globe_spins_while_connecting_and_faces_the_server_once_connected() {
        let mut state = state(
            Status::Connecting {
                spoke_id: "au3".into(),
            },
            None,
        );
        state.locations = Locations::Loaded(recorded_locations().unwrap());
        assert_eq!(target(&state), Target::Spin);

        state.helper = HelperState::Running(connected("au3"));
        assert_eq!(
            target(&state),
            Target::Place(Coordinates {
                lon: 115.8589,
                lat: -31.9522
            })
        );

        state.helper = HelperState::Running(connected("gone1"));
        assert_eq!(target(&state), Target::Home);

        state.helper = HelperState::Running(Status::Disconnecting {
            spoke_id: "au3".into(),
        });
        assert_eq!(target(&state), Target::Home);
        state.helper = HelperState::NotRunning;
        assert_eq!(target(&state), Target::Home);
    }

    #[test]
    fn the_server_ends_up_clear_of_the_status_icon() {
        let perth = Coordinates {
            lon: 115.8589,
            lat: -31.9522,
        };
        let start = Instant::now();
        let mut globe = Globe::new(Target::Spin, start, false);
        globe.set_target(Target::Place(perth), start + Duration::from_secs(2), false);
        let scene = globe.motion.scene(start + Duration::from_secs(10));
        assert!(!scene.moving);

        for scale in [1., 1.25, 1.5, 2.] {
            let (x, y) = canvas(scale, scene.pose).project(perth).unwrap();
            assert!(
                (x / scale - ZOOMED_CENTRE.0).abs() < 0.01
                    && (y / scale - ZOOMED_CENTRE.1).abs() < 0.01,
                "({x}, {y}) at {scale}x"
            );
        }
    }
}
