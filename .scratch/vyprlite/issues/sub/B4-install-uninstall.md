# B4: `vyprlite-helper install | uninstall` (rest of 07)

Owned: new `crates/helper/src/windows/install.rs`, the `install`/`uninstall` subcommand dispatch in `crates/helper/src/windows/mod.rs` (only that), and the install section of `README.md`.

- `install` (run elevated by the user, once): register `VyprLiteHelper` as an own-process, manual-start service running as LocalSystem with `--service --owner-sid <installing user's SID>`; grant the installing user start and query rights on the service (service security descriptor; `sc.exe sdset` via `Command` is acceptable if `windows-service` can't); register the tray at login unelevated via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value `VyprLite` = `"<dir>\VyprLite.exe"` (`winreg`). Paths come from the helper exe's own dir. Safe to run twice (updates in place).
- `uninstall`: ask a running helper to disconnect (IPC `disconnect`; "not running" is fine, other errors are reported), stop and delete the service, delete the Run value, delete the helper data dir (LocalSystem's `%LOCALAPPDATA%\VyprLite`, i.e. the systemprofile path; derive it, don't hardcode the drive) and stored credentials. Safe to run twice.
- Not elevated: fail with a clear "run as administrator" error.
- Model every step as pure plan data (an ordered list of steps with their arguments) built from the inputs (exe dir, SID, data dir), unit-tested; the executor is thin.
- README: replace the manual `sc.exe` instructions with the two commands.

Verify: `cargo fmt --all --check`; `cargo clippy -p vyprlite-helper --all-targets -- -D warnings`; `cargo test -p vyprlite-helper install` covering the install and uninstall plans (exact service args, SDDL with the SID, quoted Run value path) and the idempotence decisions (exists vs not). Do NOT run install or uninstall.
