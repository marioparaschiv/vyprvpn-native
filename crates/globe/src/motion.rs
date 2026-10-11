//! How the globe moves with the connection: at rest on the home view, spinning while connecting, and
//! turned and zoomed onto the server once connected.

use std::time::{Duration, Instant};

use crate::{Coordinates, HOME};

/// How fast the globe turns while connecting, in degrees a second. It turns west to east, as the
/// Earth does, so the longitude facing the viewer falls.
pub const SPIN_SPEED: f32 = 40.0;
const SPIN_UP: Duration = Duration::from_millis(600);

/// How much larger the globe is drawn once it faces the server.
pub const ZOOM: f32 = 1.8;
const ZOOM_IN: Duration = Duration::from_millis(900);
const ZOOM_OUT: Duration = Duration::from_millis(600);
/// Turns that would be faster than this on average, in degrees a second, take longer instead.
const TURN_SPEED: f32 = 240.0;

/// The marker grows over the end of the zoom in, and shrinks away when the globe moves off it.
const MARKER_IN: Duration = Duration::from_millis(300);
const MARKER_OUT: Duration = Duration::from_millis(150);

/// Where the globe is headed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Home,
    Spin,
    /// Faces and marks a place, zoomed in.
    Place(Coordinates),
}

impl Target {
    fn rest(self) -> Pose {
        match self {
            Self::Home | Self::Spin => Pose {
                facing: HOME,
                zoom: 1.0,
            },
            Self::Place(place) => Pose {
                facing: place,
                zoom: ZOOM,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// The point facing the viewer.
    pub facing: Coordinates,
    /// The globe's radius relative to the home view's.
    pub zoom: f32,
}

/// A dot on the server's location.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marker {
    pub at: Coordinates,
    /// From 0 (not shown) to 1 (full size).
    pub scale: f32,
}

/// What to draw at a moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scene {
    pub pose: Pose,
    pub marker: Option<Marker>,
    /// Whether the next moment will look different.
    pub moving: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Path {
    Still,
    Spin,
    /// Eases `turn` degrees of longitude, starting at `velocity` degrees a second, along with the
    /// latitude and zoom, and comes to rest.
    Turn {
        turn: f32,
        velocity: f32,
        duration: Duration,
    },
}

/// The globe's movement toward its latest target, as a pure function of time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    target: Target,
    since: Instant,
    from: Pose,
    path: Path,
    /// The marker shown when this motion began, shrinking away.
    leaving: Option<Marker>,
}

impl Motion {
    /// The globe as a newly opened popover shows it: spinning up from home while connecting, else
    /// at rest on its target.
    pub fn new(target: Target, now: Instant) -> Self {
        Self {
            path: match target {
                Target::Spin => Path::Spin,
                Target::Home | Target::Place(_) => Path::Still,
            },
            ..Self::still(target, now)
        }
    }

    /// At rest on the target straight away, for reduced motion.
    pub fn still(target: Target, now: Instant) -> Self {
        Self {
            target,
            since: now,
            from: target.rest(),
            path: Path::Still,
            leaving: None,
        }
    }

    pub fn target(&self) -> Target {
        self.target
    }

    /// Heads for `target` from wherever the globe is at `now`, carrying on its spin.
    pub fn retarget(self, target: Target, now: Instant) -> Self {
        if target == self.target {
            return self;
        }

        let scene = self.scene(now);
        let velocity = self.velocity(now);
        let from = Pose {
            facing: Coordinates {
                lon: wrap(scene.pose.facing.lon),
                ..scene.pose.facing
            },
            ..scene.pose
        };
        let path = match target {
            Target::Spin => Path::Spin,
            Target::Home | Target::Place(_) => {
                let shortest = if matches!(target, Target::Place(_)) {
                    ZOOM_IN
                } else {
                    ZOOM_OUT
                };
                let turn = turn(
                    from.facing.lon,
                    target.rest().facing.lon,
                    velocity,
                    shortest,
                );

                Path::Turn {
                    turn,
                    velocity,
                    duration: shortest.max(Duration::from_secs_f32(turn.abs() / TURN_SPEED)),
                }
            }
        };

        Self {
            target,
            since: now,
            from,
            path,
            leaving: scene.marker,
        }
    }

    pub fn scene(&self, now: Instant) -> Scene {
        let elapsed = now.saturating_duration_since(self.since);
        let to = self.target.rest();
        let toward = |progress: f32| Pose {
            facing: Coordinates {
                lon: self.from.facing.lon,
                lat: lerp(self.from.facing.lat, to.facing.lat, progress),
            },
            zoom: lerp(self.from.zoom, to.zoom, progress),
        };

        let (pose, arriving, moving) = match self.path {
            Path::Still => (self.from, 1.0, false),
            Path::Spin => {
                let mut pose = toward(ease_in_out(fraction(elapsed, SPIN_UP)));
                pose.facing.lon = wrap(pose.facing.lon - spun(elapsed));
                (pose, 0.0, true)
            }
            Path::Turn {
                turn,
                velocity,
                duration,
            } => {
                let progress = fraction(elapsed, duration);
                let growing = elapsed.saturating_sub(duration.saturating_sub(MARKER_IN));
                let arriving = ease_out(fraction(growing, MARKER_IN));
                if progress == 1.0 {
                    (to, arriving, false)
                } else {
                    let mut pose = toward(ease_in_out(progress));
                    let turned = hermite(progress, turn, velocity * duration.as_secs_f32());
                    pose.facing.lon = wrap(pose.facing.lon + turned);
                    (pose, arriving, true)
                }
            }
        };

        let marker = match (self.leaving, self.target) {
            (Some(leaving), _) if elapsed < MARKER_OUT => Some(Marker {
                scale: leaving.scale * (1.0 - ease_out(fraction(elapsed, MARKER_OUT))),
                ..leaving
            }),
            (_, Target::Place(at)) if arriving > 0.0 => Some(Marker {
                at,
                scale: arriving,
            }),
            _ => None,
        };

        Scene {
            pose,
            marker,
            moving: moving || (self.leaving.is_some() && elapsed < MARKER_OUT),
        }
    }

    /// How fast the facing longitude changes at `now`, in degrees a second.
    fn velocity(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.since);

        match self.path {
            Path::Still => 0.0,
            Path::Spin => -SPIN_SPEED * ease_in_out(fraction(elapsed, SPIN_UP)),
            Path::Turn {
                turn,
                velocity,
                duration,
            } => {
                let u = fraction(elapsed, duration);
                velocity * (1.0 - 4.0 * u + 3.0 * u * u)
                    + turn / duration.as_secs_f32() * 6.0 * u * (1.0 - u)
            }
        }
    }
}

/// How many degrees of longitude a spin has turned after `elapsed`: its speed eases in from rest
/// along [`ease_in_out`], then holds.
fn spun(elapsed: Duration) -> f32 {
    let ramp = SPIN_UP.as_secs_f32();
    let t = elapsed.as_secs_f32();
    if t >= ramp {
        return SPIN_SPEED * (ramp / 2.0 + t - ramp);
    }

    let u = t / ramp;
    SPIN_SPEED * ramp * (u.powi(3) - u.powi(4) / 2.0)
}

/// The longitude to travel from `from` to `to`. At rest it takes the short way round; while turning
/// it keeps turning the same way and travels far enough not to overshoot and come back.
fn turn(from: f32, to: f32, velocity: f32, duration: Duration) -> f32 {
    if velocity == 0.0 {
        return wrap(to - from);
    }

    let direction = velocity.signum();
    let ahead = (direction * (to - from)).rem_euclid(360.0);
    let coasting = velocity.abs() * duration.as_secs_f32() / 3.0;

    direction
        * if ahead < coasting {
            ahead + 360.0
        } else {
            ahead
        }
}

/// A cubic from 0 to `distance` over `u` in 0..=1, starting at `start` (in distance per unit of
/// `u`) and arriving at rest.
fn hermite(u: f32, distance: f32, start: f32) -> f32 {
    start * u * (1.0 - u) * (1.0 - u) + distance * u * u * (3.0 - 2.0 * u)
}

fn ease_in_out(u: f32) -> f32 {
    u * u * (3.0 - 2.0 * u)
}

fn ease_out(u: f32) -> f32 {
    1.0 - (1.0 - u).powi(3)
}

fn fraction(elapsed: Duration, duration: Duration) -> f32 {
    (elapsed.as_secs_f32() / duration.as_secs_f32()).min(1.0)
}

fn lerp(from: f32, to: f32, progress: f32) -> f32 {
    from + (to - from) * progress
}

/// A longitude in -180..180.
fn wrap(lon: f32) -> f32 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERTH: Coordinates = Coordinates {
        lon: 115.8589,
        lat: -31.9522,
    };
    const FRANKFURT: Coordinates = Coordinates {
        lon: 8.6765,
        lat: 50.1055,
    };
    const FRAME: Duration = Duration::from_millis(4);

    /// Scenes every 4 ms from `start` until the motion comes to rest, or for `limit` frames.
    fn scenes(motion: Motion, start: Instant, limit: u32) -> Vec<Scene> {
        let mut scenes = Vec::new();

        for frame in 0..limit {
            let scene = motion.scene(start + FRAME * frame);
            scenes.push(scene);
            if !scene.moving {
                break;
            }
        }

        scenes
    }

    /// Each frame's change in facing longitude, unwrapped across the antimeridian.
    fn steps(scenes: &[Scene]) -> Vec<f32> {
        scenes
            .windows(2)
            .map(|pair| wrap(pair[1].pose.facing.lon - pair[0].pose.facing.lon))
            .collect()
    }

    fn assert_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "{actual} is not {expected}"
        );
    }

    fn assert_rests_on(scene: Scene, place: Coordinates) {
        assert!(!scene.moving);
        assert_near(wrap(scene.pose.facing.lon - place.lon), 0.0);
        assert_near(scene.pose.facing.lat, place.lat);
        assert_near(scene.pose.zoom, ZOOM);
        assert_eq!(
            scene.marker,
            Some(Marker {
                at: place,
                scale: 1.0
            })
        );
    }

    fn spin(start: Instant) -> Motion {
        Motion::new(Target::Home, start).retarget(Target::Spin, start)
    }

    #[test]
    fn the_spin_eases_up_to_a_steady_turn_west_to_east() {
        let start = Instant::now();
        let spin = spin(start);
        let speeds: Vec<f32> = (0..=250)
            .map(|frame| -spin.velocity(start + FRAME * frame))
            .collect();

        assert_eq!(speeds[0], 0.0);
        assert!(
            speeds.windows(2).all(|pair| pair[1] >= pair[0]),
            "{speeds:?}"
        );
        assert!(
            speeds[37] < SPIN_SPEED / 5.0,
            "still slow a quarter of the way up"
        );
        assert_near(speeds[150], SPIN_SPEED);
        assert_near(speeds[250], SPIN_SPEED);

        let steps = steps(&scenes(spin, start, 251));
        assert!(steps.iter().all(|&step| step <= 0.0), "{steps:?}");
        assert_near(steps[200], -SPIN_SPEED * FRAME.as_secs_f32());
        // The ramp covers half the distance a full-speed start would.
        assert_near(spun(SPIN_UP), SPIN_SPEED * SPIN_UP.as_secs_f32() / 2.0);

        let later = spin.scene(start + FRAME * 1000);
        assert!(later.moving);
        assert_eq!(later.marker, None);
    }

    #[test]
    fn connecting_turns_and_zooms_onto_the_server_without_overshoot() {
        let start = Instant::now();
        for spinning in [300, 1700, 2600, 4100, 9000] {
            for place in [PERTH, FRANKFURT] {
                let now = start + Duration::from_millis(spinning);
                let settle = spin(start).retarget(Target::Place(place), now);
                let scenes = scenes(settle, now, 1000);

                assert_rests_on(*scenes.last().unwrap(), place);
                assert!(scenes.len() >= 226, "under 900 ms: {}", scenes.len());
                assert!(scenes.len() <= 500, "over 2 s: {}", scenes.len());
                assert!(
                    steps(&scenes).iter().all(|&step| step <= 1e-3),
                    "turned back after {spinning} ms toward {place:?}"
                );

                let zooms: Vec<f32> = scenes.iter().map(|scene| scene.pose.zoom).collect();
                assert_eq!(zooms[0], 1.0);
                assert!(zooms.windows(2).all(|pair| pair[1] >= pair[0]), "{zooms:?}");
                assert!(zooms.iter().all(|&zoom| zoom <= ZOOM));

                let lats = place.lat.min(HOME.lat)..=place.lat.max(HOME.lat);
                assert!(
                    scenes
                        .iter()
                        .all(|scene| lats.contains(&scene.pose.facing.lat))
                );
            }
        }
    }

    #[test]
    fn the_zoom_eases_in_and_out_of_its_ends() {
        let start = Instant::now();
        let settle = Motion::new(Target::Home, start).retarget(Target::Place(FRANKFURT), start);
        let zooms: Vec<f32> = scenes(settle, start, 1000)
            .iter()
            .map(|scene| scene.pose.zoom)
            .collect();

        assert_eq!(zooms.len(), 226, "900 ms at 4 ms a frame");
        assert!(zooms[1] - zooms[0] < 0.001, "starts gently");
        assert!(zooms[225] - zooms[224] < 0.001, "lands gently");
        let halfway = settle.scene(start + ZOOM_IN / 2).pose.zoom;
        assert_near(halfway, (1.0 + ZOOM) / 2.0);
    }

    #[test]
    fn the_marker_grows_in_at_the_end_and_then_everything_stops() {
        let start = Instant::now();
        let settle = Motion::new(Target::Home, start).retarget(Target::Place(PERTH), start);
        let scenes = scenes(settle, start, 1000);
        let scales: Vec<f32> = scenes
            .iter()
            .map(|scene| scene.marker.map_or(0.0, |marker| marker.scale))
            .collect();

        let first = scales.iter().position(|&scale| scale > 0.0).unwrap();
        assert!(first * 4 >= 600, "appeared at {} ms", first * 4);
        assert!(
            scales.windows(2).all(|pair| pair[1] >= pair[0]),
            "{scales:?}"
        );
        assert!(scales.iter().all(|&scale| scale <= 1.0));

        let last = *scenes.last().unwrap();
        assert_rests_on(last, PERTH);
        assert_eq!(settle.scene(start + Duration::from_secs(60)), last);
    }

    #[test]
    fn disconnecting_zooms_back_out_to_home() {
        let start = Instant::now();
        let connected = Motion::new(Target::Place(PERTH), start);
        assert_rests_on(connected.scene(start), PERTH);

        let scenes = scenes(connected.retarget(Target::Home, start), start, 1000);
        let last = *scenes.last().unwrap();

        assert_eq!(scenes.len(), 151, "600 ms at 4 ms a frame");
        assert_eq!(last.pose, Target::Home.rest());
        assert_eq!(last.marker, None);
        assert!(
            scenes
                .iter()
                .all(|scene| (1.0..=ZOOM).contains(&scene.pose.zoom))
        );

        let scales: Vec<f32> = scenes
            .iter()
            .map_while(|scene| scene.marker.map(|marker| marker.scale))
            .collect();
        assert_eq!(scales[0], 1.0);
        assert!(
            scales.windows(2).all(|pair| pair[1] <= pair[0]),
            "{scales:?}"
        );
        assert!(scales.len() <= 38, "gone in 150 ms");
    }

    #[test]
    fn switching_location_spins_out_then_settles_on_the_new_one() {
        let start = Instant::now();
        let spin = Motion::new(Target::Place(FRANKFURT), start).retarget(Target::Spin, start);
        let later = start + Duration::from_millis(1500);
        let spinning = spin.scene(later);

        assert_eq!(spinning.pose.zoom, 1.0);
        assert_eq!(spinning.marker, None);
        let settle = spin.retarget(Target::Place(PERTH), later);
        assert_rests_on(*scenes(settle, later, 1000).last().unwrap(), PERTH);
    }

    #[test]
    fn an_open_popover_shows_the_end_state_and_only_a_spin_moves() {
        let now = Instant::now();

        assert_rests_on(Motion::new(Target::Place(PERTH), now).scene(now), PERTH);
        assert!(!Motion::new(Target::Home, now).scene(now).moving);
        assert!(Motion::new(Target::Spin, now).scene(now).moving);

        let spinning = Motion::new(Target::Spin, now);
        assert_eq!(spinning.retarget(Target::Spin, now + FRAME), spinning);
    }

    #[test]
    fn reduced_motion_jumps_to_each_end_state() {
        let now = Instant::now();

        for target in [Target::Home, Target::Spin, Target::Place(PERTH)] {
            let scene = Motion::still(target, now).scene(now);
            assert!(!scene.moving);
            assert_eq!(scene.pose, target.rest());
        }
        assert_rests_on(Motion::still(Target::Place(PERTH), now).scene(now), PERTH);
    }
}
