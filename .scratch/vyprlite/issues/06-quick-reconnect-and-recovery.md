## Parent

`../parent.md` (VyprLite: all-Rust GPUI tray app + privileged helper)

## What to build

Make everyday use and failure recovery smooth. The tray menu and popover offer "Connect to <last location>" when disconnected; the helper remembers the last location across restarts. The helper reconciles its state on start: if it has a recorded session (crash, reboot, service restart), it deletes the stale server-side session and reports disconnected; the tunnel never outlives the helper, so there is no "still connected" case. The tray app reconnects to the helper automatically after either process restarts and re-renders the current state.

## Acceptance criteria

- [ ] After connecting to a location and disconnecting, the menu and popover show "Connect to <that location>", and it still does after restarting both processes.
- [ ] Killing the tunnel process while connected leads the helper to report disconnected and delete the server-side session; nothing is left stuck on "Connected".
- [ ] Rebooting while connected comes back cleanly as disconnected, never in an inconsistent state. (The tunnel never outlives the helper on either platform: WireGuardNT removes the adapter with its process, and the macOS utun dies with boringtun, so there is no "tunnel survived" case.)
- [ ] Restarting the helper while the tray app runs: the tray shows "Helper not running" briefly, then the real state, without user action.
- [ ] Reconnecting to the helper uses a backoff, not a busy loop; idle CPU stays effectively 0% while the helper is down.
- [ ] A test covers the reconciliation logic for each case: no session, recorded session, failed deletion retried.

## Blocked by

- `04-connect-disconnect.md`
