# E2: Popover enter and exit animation

Owned: the popover's open/close presentation in `crates/tray/src/popover/mod.rs` and a small pure module for the curve if needed. Starts after E1 is committed.

- Enter: a modern web-style "blur scale in": the card scales up from about 0.96 to 1.0, fades from 0 to 1 and un-blurs from about 8 px to 0 (like `filter: blur()` on the web), over 160 to 200 ms with a strong ease-out (e.g. cubic-bezier(0.16, 1, 0.3, 1)). The scale origin is the side facing the tray icon (bottom-right for a bottom-right taskbar, top-centre under a macOS menu-bar icon), so it grows out of the icon.
- Exit: faster and simpler, about 120 ms fade (with a slight scale-down to 0.98 if cheap), then the window closes. Esc, a second tray click and focus loss all use it; a new open during the exit cancels it cleanly.
- Reduced motion: no scale; a short fade or instant.
- GPUI first: use GPUI/gpui-kit animation, transform and blur support if it can scale and blur the card (check for element blur, backdrop blur, or rendering the card to an image and blurring it).  If blur is impossible in GPUI without unsafe or heavy per-frame work, say so and ship scale + fade only. If GPUI can't scale arbitrary element trees, say so and pick the closest honest alternative (e.g. fade plus a small slide toward the icon of about 8 px, which GPUI can do by animating the offset), rather than faking scale by resizing and reflowing the content.
- Frames run only during the animation; idle CPU stays ~0%.
- Pure, tested: the origin side as a function of the placement edge, and the progress curve values (start, end, monotonic, no overshoot past 1.0 opacity).

Verify: `cargo fmt --all --check`; `cargo clippy -p vyprvpn-tray --all-targets -- -D warnings`; `cargo test -p vyprvpn-tray`. Commit as `feat(tray): ...`, no AI attribution. The user judges the feel by hand.
