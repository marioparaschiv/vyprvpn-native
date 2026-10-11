## Parent

`../parent.md` (VyprVPN: all-Rust GPUI tray app + privileged helper)

## What to build

VyprVPN on macOS, end to end. The helper runs as a root launchd daemon, uses userspace `boringtun` on a `utun` interface, and configures the interface address, routes (including keeping the WireGuard endpoint reachable outside the tunnel), and DNS itself, undoing all of it on disconnect. It serves the same IPC protocol over a Unix domain socket restricted to root and the installing user, and stores credentials per the parent **Decisions**. API calls send `X-GF-PLATFORM: Mac`.

The same Rust + GPUI tray app runs as a menu-bar item: native right-click menu, and the issue 05 popover anchored directly below the menu-bar icon, horizontally centred on it and clamped to the screen. This needs a Mac to build and verify; it is built locally and not notarized.

## Acceptance criteria

- [ ] On a Mac: install once with the admin password; the menu-bar icon, location menu, login, connect, disconnect, and popover all work as on Windows.
- [ ] While connected, the public IP is the VPN's and DNS queries go to the VPN's DNS servers.
- [ ] Disconnect restores the original routes and DNS exactly; nothing leaks or remains after a disconnect, a helper crash, or a reboot.
- [ ] The popover appears directly below the menu-bar icon on the built-in display and on an external display.
- [ ] Idle CPU of both processes is effectively 0% while connected and while disconnected.
- [ ] The issue 05 placement function has macOS test cases (menu-bar anchoring, clamping at the right screen edge).

## Blocked by

- `04-connect-disconnect.md`
