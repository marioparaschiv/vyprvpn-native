# C2: Tray connect, disconnect and live status (tray side of 04)

Owned: `crates/tray/src/tray_menu.rs`, `crates/tray/src/helper_link.rs`, new `crates/tray/src/icons.rs` (or `assets/`), `crates/tray/src/main.rs` lines for these. Starts after B1 lands.

- Native menu: status line, Connect (to the selected location, defaulting to `last_spoke_id`, else the first location; label "Connect to <location>"), Disconnect, Log in… / Log out, Exit. Connect and Disconnect are disabled while busy or logged out.
- Status line and tooltip: "Disconnected", "Connecting to Germany…", "Connected: Germany", "Disconnecting…", using location names from the cached locations list.
- Tray icon colour: grey disconnected, light blue busy, pastel green connected. Generate the three icons at build time or embed small PNGs; no runtime drawing loop.
- A `disconnected` status with `error` shows a native tray notification with the message, once per error.
- Expose the selected location and a `connect(spoke_id)` / `disconnect()` API on the helper link, so the popover (D1) reuses the same path.
- Pure, unit-tested mapping: `Status` + locations + account -> menu labels, enabled states, icon kind, tooltip, notification.

Verify: `cargo fmt --all --check`; `cargo clippy -p vyprlite-tray --all-targets -- -D warnings`; `cargo test -p vyprlite-tray` with the mapping cases for every status, logged out, and a missing location name. Idle CPU ~0% (measure against the console helper with fakes, report).
