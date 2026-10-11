## Parent

`../parent.md` (VyprVPN: all-Rust GPUI tray app + privileged helper)

## What to build

Connect and disconnect. The native tray menu has a Connect item (to the selected location, which defaults to the last-used one; until issue 05 adds the picker, the first location or the last-used one) and a Disconnect item; issue 05's popover uses the same helper path. Connecting asks the helper to connect: it generates a WireGuard key pair, creates a session via the connect endpoint, and brings up the tunnel itself: the helper creates a WireGuardNT adapter via `wireguard.dll` (the `wireguard-nt` crate, loaded from next to the exe), sets the address, MTU 1420, the split default routes (`0.0.0.0/1`, `128.0.0.0/1`) and DNS. The WireGuardNT driver keeps the peer endpoint reachable outside the tunnel by itself (no routing loop, no extra endpoint route). A "Disconnect" item stops the tunnel and deletes the session. Connecting while already connected switches location (disconnect, then connect).

The helper pushes status events and the tray app reflects them live: icon colour (grey disconnected, light blue busy, pastel green connected), tooltip, and status line ("Connecting to Germany…", "Connected: Germany"). Connect and Disconnect are disabled while busy or logged out. Errors appear as a tray notification carrying the helper's message.

## Acceptance criteria

- [ ] Connect connects: the public IP changes to the VPN's, the status shows "Connected: <location>", and the icon turns green.
- [ ] Disconnect restores the normal connection, deletes the server-side session, removes the adapter, and leaves no private key on disk.
- [ ] Switching location while connected ends with exactly one active session and one tunnel.
- [ ] Closing the tray app leaves the VPN connected; reopening it shows the correct connected state.
- [ ] An API or tunnel failure shows a notification with the reason and leaves the state consistent (never stuck on "Connecting…").
- [ ] Idle CPU of both processes while connected is effectively 0% (no polling loops; status is event-driven).
- [ ] A test turns a realistic connect response into the adapter's interface and peer config (keys, PSK, endpoint, allowed IPs, address, DNS).

## Blocked by

- `02-locations-in-tray-menu.md`
- `03-login-logout.md`
