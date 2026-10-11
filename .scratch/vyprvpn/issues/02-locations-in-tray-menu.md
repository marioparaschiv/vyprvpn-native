## Parent

`../parent.md` (VyprVPN: all-Rust GPUI tray app + privileged helper)

## What to build

The helper fetches the location list from the Vypr locations endpoint (see parent **Vypr API facts**) and serves it over IPC. The tray app groups it Region → Country → City (a City level only when a country has several locations; the city label is the location name with the "Country - " prefix removed), ready for the issue 05 popover, which is the only location picker. The native right-click menu does not list locations: Windows' native popup menus can't show a clear hover highlight without more unsafe code, so the menu stays minimal (status line, account item, Exit; Connect/Disconnect come with issue 04).

Build the menu so it can be updated in place (enable/disable, relabel) without being torn down while it is open.

The tray app caches country flags for the popover. Flags come from `https://api.vyprvpn.com/images/vpn_flags/64/{country_code lowercase}.png` using the location's 3-letter `country_code`, downloaded once by the tray app and cached on disk in `%LOCALAPPDATA%\VyprVPN\flags` (macOS: `~/Library/Caches/VyprVPN/flags`), keyed by code. A missing or failed flag shows a neutral placeholder and never blocks the UI.

## Acceptance criteria

- [ ] All live locations (~73) are grouped Region → Country (→ City when a country has more than one), sorted alphabetically at each level.
- [ ] The tray app tracks loading, loaded and failed states for the list; a network failure carries a readable error and is retried with a short backoff.
- [ ] The helper caches the list so the tray app does not refetch every time.
- [ ] A test feeds a realistic locations payload through the grouping and asserts the Region/Country/City structure, including a multi-city country (e.g. Australia).

## Blocked by

- `01-helper-and-tray-status.md`
