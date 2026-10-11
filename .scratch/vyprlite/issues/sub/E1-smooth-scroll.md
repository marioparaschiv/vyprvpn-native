# E1: Safari-style smooth scrolling in the popover's location list

Owned: the scroll handling in `crates/tray/src/popover/location_list.rs` and a new pure module for the easing (e.g. `crates/tray/src/popover/smooth_scroll.rs`). Starts after the review commits land.

- Mouse-wheel input that arrives in line steps (a notched wheel) animates the list's scroll offset toward the new target instead of jumping: ease-out (a critically damped spring or cubic ease-out, about 200 to 250 ms), and further wheel notches during an animation retarget smoothly from the current position and velocity, the way Safari does.
- Precise pixel deltas (touchpads, high-resolution wheels) pass through 1:1, with no added easing, so they keep the OS's own momentum.
- The animation clamps at the list's top and bottom, with no overshoot.
- Respects reduced motion: with it on, wheel steps jump instantly.
- Frames are requested only while an animation is running (GPUI animation frames, no timers or polling), so idle CPU stays ~0%.
- Implement it once, generic over a GPUI scroll handle, so any other scrollable view can reuse it. If gpui-kit or GPUI already offers smooth wheel scrolling, use that instead and say so.
- The easing and retargeting math is a pure, unit-tested function of (current offset, velocity, target, elapsed time).

Verify: `cargo fmt --all --check`; `cargo clippy -p vyprlite-tray --all-targets -- -D warnings`; `cargo test -p vyprlite-tray smooth_scroll` (ease-out reaches the target with no overshoot, retarget mid-animation keeps velocity continuous, clamping at the bounds, reduced motion jumps instantly, pixel deltas pass through). Idle CPU ~0% after scrolling stops (measure in release, report). The user checks the feel by hand.
