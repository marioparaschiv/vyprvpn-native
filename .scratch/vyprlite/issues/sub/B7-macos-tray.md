# B7: macOS tray app (part of 08)

Owned: new `crates/tray/src/macos/**`, the macOS `#[cfg]` lines in `crates/tray/src/main.rs` and `crates/tray/src/trust.rs`, and macOS target deps in `crates/tray/Cargo.toml`.

- The tray runs as a menu-bar item with `tray-icon` + `muda`, on GPUI's main loop as on Windows.
- Helper start: no SCM; connecting to `/var/run/vyprlite.sock` is enough (launchd starts the helper). The status line gives a clear reason if the socket is missing.
- Trust check: the socket peer must be root (uid 0), or the current user in console mode (peer credentials via `interprocess`), through the existing seam in `crates/tray/src/trust.rs`, without changing Windows behaviour.
- Flag cache dir `~/Library/Caches/VyprLite/flags`. The theme follows macOS appearance (GPUI) and the accent if a safe API exists; otherwise system blue, and report it.
- Reuse every platform-neutral module; don't fork them.

Verify: `cargo fmt --all --check`; Windows still green: `cargo clippy -p vyprlite-tray --all-targets -- -D warnings`, `cargo test -p vyprlite-tray`; macOS: `cargo check --target aarch64-apple-darwin -p vyprlite-tray`. Report exactly which crates can't cross-check without the Apple SDK.
