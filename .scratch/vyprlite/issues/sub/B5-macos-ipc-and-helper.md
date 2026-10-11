# B5: macOS IPC transport and helper daemon (part of 08)

Owned: new `crates/ipc/src/macos.rs`, new `crates/helper/src/macos/**`, new `crates/helper/macos/` (launchd plist), and the macOS `#[cfg]` `mod`/`pub use` lines in `crates/ipc/src/lib.rs` and `crates/helper/src/main.rs` (replace the helper's Windows-only `compile_error!` with one for "neither Windows nor macOS").

- IPC: Unix socket `/var/run/vyprlite.sock` via `interprocess` (the API family the Windows side uses); console mode uses a per-user socket path. The helper accepts only root and the installing user's uid (passed like `--owner-sid` on Windows), checked via peer credentials.
- Daemon: launchd socket activation if a maintained safe crate supports `launch_activate_socket`; otherwise launchd starts it on demand and it listens itself. Report which. Same exit-when-idle behaviour as Windows. Data dir `/Library/Application Support/VyprLite`, mode 0700, root-owned. Credentials: root-only file in that dir. Panics to `crash.log`. API calls send `X-GF-PLATFORM: Mac`: if `vypr-api` has no platform parameter, message the lead instead of editing it.
- Tunnel: use `crates/tunnel`'s macOS API; persist `ActiveTunnel::teardown()` right after `up` so recovery can replay it (the helper's connect logic is another agent's; expose what it needs through the platform module and report).
- Reuse the platform-neutral helper modules (`server`, `connection`, `state`); don't fork them.

Verify: `cargo fmt --all --check`; Windows still green: `cargo clippy -p ipc -p vyprlite-helper --all-targets -- -D warnings`, `cargo test -p ipc -p vyprlite-helper`; macOS: `rustup target add aarch64-apple-darwin`, then `cargo check --target aarch64-apple-darwin -p ipc -p vyprlite-helper` and clippy for that target. If `ring` can't build without the Apple SDK, report it and type-check with a stub as t3 did.
