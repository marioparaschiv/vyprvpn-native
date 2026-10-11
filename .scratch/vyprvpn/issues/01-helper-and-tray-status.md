## Parent

`../parent.md` (VyprVPN: all-Rust GPUI tray app + privileged helper)

## What to build

The thinnest end-to-end path through the new architecture on Windows. The Rust helper runs as a Windows service and serves the IPC channel described in the parent's **Architecture** section. A new Rust tray app starts unelevated, connects to the helper, asks for status, and shows it as a disabled status line ("Disconnected") at the top of a native right-click tray menu, with an Exit item. If the helper is not reachable, the status line says so.

The helper runs only when needed (see parent **Helper lifecycle**): the tray app starts the manual-start service through the Service Control Manager when it launches or finds the helper gone, and the helper exits by itself once no tray app is connected and no tunnel is up. Startup is fast: the helper accepts IPC connections before doing any other work.

Set up the Cargo workspace: `crates/vypr-api` (Vypr HTTP client), `crates/ipc` (protocol types shared by both halves), `crates/tunnel` (WireGuard keys, config model, OS backend), `helper/` (binary `vyprvpn-helper`) and `tray/` (binary `VyprVPN`). One `cargo build --workspace` builds both halves. The helper also runs in a `--console` foreground mode for unelevated development and testing.

## Acceptance criteria

- [ ] The helper installs and runs as a Windows service, and its IPC endpoint accepts connections only from SYSTEM and the installing user.
- [ ] The tray app runs without elevation, shows a tray icon, and its right-click menu shows "Disconnected" and Exit.
- [ ] Launching the tray app starts the stopped helper service, unelevated, and shows "Disconnected".
- [ ] Stopping the helper service while the tray runs shows "Helper not running" within a few seconds; the tray starts it again with a backoff and returns to "Disconnected" without being restarted.
- [ ] Exiting the tray app while disconnected makes the helper exit by itself.
- [ ] The helper accepts IPC connections within 100 ms of process start (measured in `--console` mode, release build); no network or tunnel work happens before the pipe is listening.
- [ ] The status request/response and the status event have a written protocol definition shared by both halves (the `ipc` crate's types, documented in `docs/ipc-protocol.md`), with a test on each side that round-trips a status message.
- [ ] The workspace builds with one documented command and `cargo test --workspace` passes.

## Blocked by

None - can start immediately
