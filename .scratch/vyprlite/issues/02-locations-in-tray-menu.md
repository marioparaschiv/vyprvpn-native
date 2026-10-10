## Parent

`../parent.md` (VyprLite: all-Rust GPUI tray app + privileged helper)

## What to build

The helper fetches the location list from the Vypr locations endpoint (see parent **Vypr API facts**) and serves it over IPC. The tray app shows it in the native right-click menu as Region → Country submenus; a country with several locations gets a further City submenu (the city label is the location name with the "Country - " prefix removed). Until the list arrives the menu shows a disabled "Loading locations…" item; on failure it shows the error. Items do not connect yet (that is issue 04), but they must exist as real menu items tied to a location's `spoke_id`.

Build the menu so it can be updated in place (enable/disable, relabel) without being torn down while it is open.

Native menu items show the country flag as a menu icon (`muda` `IconMenuItem`). Flags come from `https://api.vyprvpn.com/images/vpn_flags/64/{country_code lowercase}.png` using the location's 3-letter `country_code`, downloaded once by the tray app and cached on disk in `%LOCALAPPDATA%\VyprLite\flags` (macOS: `~/Library/Caches/VyprLite/flags`), keyed by code. A missing or failed flag shows a neutral placeholder and never blocks the UI.

## Acceptance criteria

- [ ] All live locations (~73) appear, grouped Region → Country (→ City when a country has more than one), sorted alphabetically at each level.
- [ ] "Loading locations…" shows until the list arrives; a network failure shows a readable error and is retried with a short backoff.
- [ ] The helper caches the list so opening the menu does not refetch every time.
- [ ] A test feeds a realistic locations payload through the grouping and asserts the Region/Country/City structure, including a multi-city country (e.g. Australia).

## Blocked by

- `01-helper-and-tray-status.md`
