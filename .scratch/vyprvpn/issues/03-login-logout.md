## Parent

`../parent.md` (VyprVPN: all-Rust GPUI tray app + privileged helper)

## What to build

Log in and log out end to end. The tray menu shows "Log in…" when logged out and "Log out (<username>)" when logged in. "Log in…" opens a small GPUI login window (username, password, Log in, Cancel; Enter submits, Esc cancels). The tray app sends the credentials to the helper, which validates them against the settings endpoint (see parent **Vypr API facts**), stores them encrypted (see parent **Decisions**), and reports the logged-in username. Wrong credentials show the error in the login window without closing it. On first start with no stored login, the login window opens automatically.

Log out asks the helper to delete the stored credentials, and also to disconnect any active session once issue 04 exists.

## Acceptance criteria

- [ ] Correct credentials close the window and the menu shows "Log out (<username>)"; the login survives restarting both the tray app and the helper.
- [ ] A 403 from Vypr shows "invalid username or password" inline; the window stays open with the username kept.
- [ ] The password never appears in logs, crash output, or IPC events; it travels only in the single login request.
- [ ] Stored credentials are readable only by the helper, not by the unelevated user directly.
- [ ] Log out removes the stored credentials and the menu returns to "Log in…".
- [ ] Tests cover the `X-GF-Agent` digest for a known username/password pair and the helper's mapping of a 403 to an "invalid credentials" error.

## Blocked by

- `01-helper-and-tray-status.md`
