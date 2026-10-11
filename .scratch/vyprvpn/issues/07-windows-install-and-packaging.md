## Parent

`../parent.md` (VyprVPN: all-Rust GPUI tray app + privileged helper)

## What to build

A usable Windows install. One build command produces a distributable folder containing the tray app, the helper, and the official signed `wireguard.dll` (see parent **References**). The build fetches the official `wireguard.dll` and checks it against the pinned hash and signature; the DLL is never committed. A single elevated install step registers the helper service as manual-start and grants the installing user start and query rights on it (see parent **Helper lifecycle**), restricts the IPC endpoint to the installing user (see parent **Architecture**), and registers the tray app to start at login unelevated. An uninstall step disconnects, removes the service, the autostart entry, the data directory, and stored credentials.

## Acceptance criteria

- [ ] From a clean checkout, one command produces the distributable folder; the build fails if `wireguard.dll` does not match the pinned hash or signature.
- [ ] Running install once (one UAC prompt) results in: helper service registered (not running at boot), tray app starting at the next login without any UAC prompt and starting the helper itself.
- [ ] After a reboot, the tray icon appears and the menu works with no elevation prompt.
- [ ] Uninstall leaves no service, no autostart entry, no data directory, and no stored credentials, and the normal network works.
- [ ] Install and uninstall are both safe to run twice.

## Blocked by

- `04-connect-disconnect.md`
