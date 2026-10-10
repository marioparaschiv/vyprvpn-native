## Parent

`../parent.md` (VyprLite: all-Rust GPUI tray app + privileged helper)

## What to build

The thinnest end-to-end path through the new architecture on Windows. The Rust helper runs as a Windows service and serves the IPC channel described in the parent's **Architecture** section. A new Rust tray app starts unelevated, connects to the helper, asks for status, and shows it as a disabled status line ("Disconnected") at the top of a native right-click tray menu, with an Exit item. If the helper is not reachable, the status line says so.

Set up the Cargo workspace: `crates/vypr-api` (Vypr HTTP client), `crates/ipc` (protocol types shared by both halves), `crates/tunnel` (WireGuard keys, config model, OS backend), `helper/` (binary `vyprlite-helper`) and `tray/` (binary `VyprLite`). One `cargo build --workspace` builds both halves. The helper also runs in a `--console` foreground mode for unelevated development and testing.

## Acceptance criteria

- [ ] The helper installs and runs as a Windows service, and its IPC endpoint accepts connections only from SYSTEM and the installing user.
- [ ] The tray app runs without elevation, shows a tray icon, and its right-click menu shows "Disconnected" and Exit.
- [ ] Stopping the helper service changes the tray status to "Helper not running" within a few seconds, and starting it again restores "Disconnected" without restarting the tray app.
- [ ] The status request/response and the status event have a written protocol definition shared by both halves (the `ipc` crate's types, documented in `docs/ipc-protocol.md`), with a test on each side that round-trips a status message.
- [ ] The workspace builds with one documented command and `cargo test --workspace` passes.

## Blocked by

None - can start immediately
