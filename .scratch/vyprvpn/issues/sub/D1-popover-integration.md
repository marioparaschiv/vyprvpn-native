# D1: Popover integration (rest of 05, tray side of 06)

Owned: `crates/tray/src/popover/**` (after B6), the tray-icon click handling in `crates/tray/src/main.rs`, and a small conversion module for `placement` (e.g. `crates/tray/src/popover/anchor.rs`). Starts after B6 (popover view), C2 (tray connect) and B3 (placement) land.

- Left-click on the tray icon toggles the popover; a second left-click closes it; Esc or losing focus (clicking elsewhere) closes it. GPUI switches to windowed mode only while it is open and back to headless after, so idle CPU returns to ~0%.
- Position: `placement::popover_origin` from the click event's icon rect and the monitor containing it (bounds, work area, scale), converted to GPUI units at this edge. Works on any monitor.
- Live state: the popover's view model is fed from the same helper link as the menu (C2), including account and locations. Connect / Disconnect / Switch go through C2's `connect(spoke_id)` / `disconnect()`.
- The selected location defaults to `last_spoke_id`; the menu's "Connect to <location>" uses the same selection, so both always agree.
- Keep the dev flag from B6 working with fake data.

Verify: `cargo fmt --all --check`; `cargo clippy -p vyprvpn-tray --all-targets -- -D warnings`; `cargo test -p vyprvpn-tray` with tests for the toggle state (open, second click closes, Esc closes, focus loss closes), the icon-rect and monitor conversion to GPUI units at 100/125/150%, and the shared selection (picking in the popover changes the menu label). Idle CPU ~0% after closing (measure, report). Popover-window-only screenshots of the live states against the console helper with fakes (no live API).
