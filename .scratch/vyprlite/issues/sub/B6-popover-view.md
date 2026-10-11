# B6: Popover UI, standalone view (part of 05)

Owned: new `crates/tray/src/popover/**`, its `mod popover;` line in `crates/tray/src/main.rs`, and a debug-only entry in `crates/tray/src/main.rs` (`#[cfg(debug_assertions)]`) that opens the popover with fake data when `VYPRLITE_DEV_POPOVER=disconnected|connecting|connected|error|subpage` is set.

Build the GPUI popover exactly as ticket 05 "What to build" and "Visuals" describe (status zone; location row; full-width Connect button reading Connect / Disconnect / Connecting… / Switch to <location>; the subpage with back arrow, focused search, region-grouped list with flags, arrow keys and Enter; slide transitions respecting reduced-motion; no window chrome, rounded corners, shadow), with gpui-kit components and `crates/tray/src/theme.rs` (light/dark and accent, live).
- Drive it from a view model in `popover/` (state, selected spoke_id, grouped locations from the `locations` crate (B3), flag paths from `flags.rs`). The button's label and enabled state are a pure, unit-tested function of the model. The model maps 1:1 from the B1 `Status` shape (`issues/sub/B1-ipc-connection-protocol.md`); write that mapping against B1's types once they exist.
- Filtering: use the `locations` crate filter from B3 once it lands; until then keep the search wiring behind a small seam and don't write a second filter.
- Not in scope: tray-icon left-click, placement, talking to the helper (D1 does that).
- Screenshots: main page (disconnected, connecting, connected), error, and the subpage, light and dark (`VYPRLITE_THEME`), captured only from the popover's own window handle with `PrintWindow`, saved under `%TEMP%\vyprlite-shots\`. Look at each image before keeping it; delete any that shows anything but the popover.

Verify: `cargo fmt --all --check`; `cargo clippy -p vyprlite-tray --all-targets -- -D warnings`; `cargo test -p vyprlite-tray popover` (button label and enabled state for every state and selection, the model mapping, keyboard selection). Idle CPU ~0% after the popover closes (measure, report).
