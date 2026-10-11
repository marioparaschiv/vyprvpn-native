## Parent

`../parent.md` (VyprVPN: all-Rust GPUI tray app + privileged helper)

## What to build

The app's main UI. Left-clicking the tray icon toggles a polished GPUI popover anchored to the tray icon (see parent **Decisions**). Esc or clicking elsewhere closes it. It has three zones:

- **Top: connection status.** State (Disconnected / Connecting… / Connected / Error) with a coloured indicator. When connected, also the location with its flag, the tunnel IP, and the connected-for duration.
- **Middle: location selector row,** directly above the button. It shows the selected location's flag and name with a chevron. Clicking it slides to a **subpage** with a back arrow, a focused search box, and the location list grouped by region with flags. Typing filters by location name, country, country code, region, or `spoke_id`, case-insensitively. Arrow keys move the selection and Enter picks it. Picking a location returns to the main page with it selected; it doesn't connect by itself.
- **Bottom: a full-width Connect button.** It becomes Disconnect when connected, and is disabled with a spinner and "Connecting…" while busy. When connected to a different location than the one selected, it reads "Switch to <location>". Connecting goes through the same helper path as issue 04.

The selected location defaults to the last-used one (issue 06 persists it). Flags come from the tray app's on-disk flag cache (see issue 02); a missing flag shows a neutral placeholder.

Visuals:

- follows the OS light/dark mode and accent colour, switching live when they change (shared theme logic from issue 02/03)
- rounded corners and a subtle shadow, consistent spacing
- short page-slide and state transitions that respect reduced-motion
- no native window chrome

Placement uses the icon rectangle reported by the tray click event. On Windows, place the popover next to the icon on whichever screen edge the taskbar is on, kept inside the monitor's work area, converting between physical pixels and GPUI's scaled units. macOS placement is issue 08, so write the placement as one pure function taking the icon rect, monitor bounds, work area, and popover size, and returning the popover origin.

## Acceptance criteria

- [ ] Left-click opens the popover against the tray icon with the taskbar at the bottom, top, left, and right.
- [ ] Placement is correct at 100%, 125%, and 150% display scaling, and on a secondary monitor.
- [ ] With the icon in the Windows 11 hidden-icons overflow, the popover still opens fully on-screen.
- [ ] Clicking outside or pressing Esc closes it; a second left-click toggles it closed.
- [ ] On the subpage, typing "perth", "GER", or "de1" narrows the list to the matching location; Enter returns to the main page with it selected, and Connect connects to it.
- [ ] The Connect button reads Connect / Disconnect / "Connecting…" (disabled, spinner) / "Switch to <location>" according to state and selection.
- [ ] Unit tests cover the placement function for all four taskbar edges and for clamping at screen corners, plus the filter function.
- [ ] Screenshots of the main page (disconnected, connecting, connected) and of the subpage, in light and dark mode, are attached to the report.

## Blocked by

- `04-connect-disconnect.md`
