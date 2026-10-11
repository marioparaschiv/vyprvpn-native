# E6: Globe spins while connecting, then zooms to a green dot

Owned: `crates/globe/**`, `crates/tray/src/popover/globe.rs`, and the globe's wiring in `crates/tray/src/popover/mod.rs`.

Behaviour, driven by the popover's connection state:
- Disconnected: static, as now (Europe/Atlantic view).
- Connecting: the globe spins up from rest (ease-in over ~600 ms) to a steady rotation about its axis (~40 degrees/s, west to east), and keeps spinning while connecting.
- Connected: the spin eases out and the globe turns so the connected server's location faces the viewer, while zooming in (radius ~1.8x, over ~900 ms with a smooth ease-in-out), and a green dot appears at the server's location: the tray's connected colour (pastel green #8EE0A1) with the same darker ring style as the tray icon, scaling in at the end. Then it stops: static, no further frames.
- Opening the popover while already connected shows the final zoomed view with the dot, no animation.
- Disconnecting or an error: zoom back out to the default static view (~600 ms).
- Switching location while connected: spin-and-zoom from the current view to the new location.
- Reduced motion: no spin or zoom animation; jump straight to each end state.

Data:
- A table of coordinates for every location (lat/lon of the server city), keyed by `spoke_id`, covering every entry in the recorded locations fixture, with a fallback by country code (country centroid) for unknown spoke ids, and no dot if neither is known. Embedded, no network.

Rendering:
- The dithered image must look stable while turning: dither in screen space but keep dots aligned to the device pixel grid; no shimmer from sub-pixel offsets.
- Frames only while an animation runs (GPUI animation frames, no timers or polling); measure the per-frame render cost in release and keep it under ~4 ms at 150% scale (lower the frame rate or precompute per-frame land lookups if needed); idle CPU stays ~0% once static.
- Pure, tested: the animation timeline (spin-up velocity curve, the final orientation centres the target lat/lon, zoom curve endpoints, no overshoot), the coordinate table (every fixture spoke_id has coordinates within its country's bounds), and the projection with zoom (target point lands at the centre).

Verify: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo xtask dist`. Write frames of the connecting spin and the final zoomed view with the dot (e.g. connected to Germany and to Perth) as PNGs to `%TEMP%\vyprlite-globe-motion\`, look at them yourself, and report per-frame render time and idle CPU. Commit as `feat(tray): ...`, no AI attribution, don't push.
