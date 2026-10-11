# F1: Popover stays open while used; install only from an admin-only folder

Owned: `crates/tray/src/popover/mod.rs`, `crates/tray/src/popover/toggle.rs`, the popover open/close code in `crates/tray/src/main.rs`; `crates/helper/src/windows/install.rs` and the README install steps.

## 1. Popover closes while the user interacts with it (user-reported, installed build)
Clicking inside the popover, opening the location list, typing in search, or pressing Connect closes it. It must stay open for any interaction inside it and close only on Esc, a second tray-icon click, or a click outside VyprLite.
- Find the root cause before changing anything. Candidates: the popover opens without becoming the foreground window (so the first click inside triggers an activation change that the close-on-deactivate handler treats as focus loss); the handler reacting to focus moves inside the window (search box, list, subpage slide) instead of window deactivation; the 500 ms toggle debounce misfiring; the login-window lookup or `activate_window` calls in `main.rs` stealing activation.
- Windows foreground rules: a process that just received the tray click may take the foreground; activate the popover window on open (GPUI `cx.activate(true)` / `window.activate_window()`), then close only on real window deactivation.
- Tests: "interaction inside doesn't close" (focus moves, subpage, typing, button press) next to `losing_focus_closes_the_popover`.

## 2. Install folder must be admin-only (security)
`install` registers the service to run `vyprlite-helper.exe` as LocalSystem from the exe's own folder, and the tunnel loads `wireguard.dll` from it. A user-writable folder (e.g. the repo's `dist\`) lets a normal user swap in code that runs as SYSTEM.
- `install` refuses, with a typed error and a clear message, unless the folder and both files can only be written by SYSTEM, Administrators or TrustedInstaller (Program Files qualifies). Read the effective ACL safely (a maintained safe crate, or parse `icacls` output via `Command`), and test the decision as a pure function over parsed ACL entries (Program Files-like ACL passes; Users/Authenticated Users/Everyone/the user's own SID with write or modify fails).
- README: copy `dist\` to `C:\Program Files\VyprLite` first, then run `install` from there.

Verify: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`. Commit each part separately (`fix(tray): ...`, `fix(helper): ...`), no AI attribution, don't push. The popover fix needs the user's hand check; say so.
