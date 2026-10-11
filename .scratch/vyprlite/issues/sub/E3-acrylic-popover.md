# E3: Acrylic (blurred) background for the popover

Owned: `crates/tray/src/popover/**` (window options, card styling), `crates/tray/src/theme.rs` (translucent surface colours), and `crates/tray/src/popover/foreground.rs`-style Windows helpers if needed.

The popover's background shows a blurred, tinted view of what's behind it, like Windows 11 flyouts (the network/volume flyouts) and macOS menus.
- Use GPUI's own support first: `WindowBackgroundAppearance::Blurred` (check how gpui-fast-windows implements it: DWM system backdrop `DWMWA_SYSTEMBACKDROP_TYPE` acrylic vs. the older accent policy, and its macOS equivalent). If GPUI offers a Windows 11 acrylic/transient backdrop, prefer that over the legacy blur.
- The card's surface becomes translucent (theme surface colour with alpha, tuned separately for light and dark so text stays readable, roughly like Windows 11 acrylic tint) so the blur shows through; text, the location row and the button stay opaque.
- The blur must cover exactly the rounded card, not a square window with a transparent shadow margin. Likely: drop the 16 px shadow margin, size the window to the card, and use the system's rounded corners and shadow (`DWMWA_WINDOW_CORNER_PREFERENCE` = round on Windows 11; GPUI or winsafe, no new unsafe). Re-check placement (`Popover::window_size`, anchor) after removing the margin.
- Fallback: when Windows "Transparency effects" is off, Battery Saver disables it, or the OS doesn't support it (Windows 10), use the current solid card. Follow the setting live if there's an event-driven way; otherwise read it on each open.
- Keep the E2 fade and slide working; say how the backdrop behaves during the fade (DWM backdrops don't fade with content).
- No new unsafe. If acrylic needs an API with no safe wrapper, stop and report.
- macOS: use GPUI's Blurred background (vibrancy) the same way; type-check with b7's stub script.

Verify: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`. Tests for the fallback decision and the window size/placement without the margin. Commit as `feat(tray): ...`, no AI attribution. Popover-window-only screenshots, light and dark, if PrintWindow captures the backdrop (say if it doesn't). The user judges the look by hand.
