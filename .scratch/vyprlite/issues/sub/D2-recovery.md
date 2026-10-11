# D2: Helper recovery on start (helper side of 06)

Owned: `crates/helper/src/` recovery code (a new `recovery.rs` plus its call site) and the platform hooks it needs. Starts after C1 (helper connect) lands.

- First, before serving any connect request but after the pipe is listening: move a leftover `session` from `connection.json` into `pending_deletions` (a quick local file write), because C1 overwrites `session` on the next connect. Then, without blocking requests, reconcile:
  - no session: report `disconnected` with the persisted `last_spoke_id`.
  - pending server-session deletions recorded by C1 (a delete that failed while online or offline): retry each with a capped backoff, without blocking requests; remove it once deleted.
  - session recorded, no tunnel (always the case on Windows after a helper restart, because WireGuardNT removes the adapter when its process exits): delete the server-side session (B2 `disconnect`), clear the record, report `disconnected`. If the delete fails (offline), keep the record and retry with backoff, without blocking requests.
  - macOS: replay the persisted `Teardown` before reporting disconnected.
- `last_spoke_id` survives restarts.
- The tray already reconnects to a restarted helper with backoff (ticket 01); confirm it re-renders the recovered state, no change expected.

Verify: `cargo fmt --all --check`; `cargo clippy -p vyprlite-helper --all-targets -- -D warnings`; `cargo test -p vyprlite-helper recovery` with fakes for each case: no session; session without tunnel (deleted, cleared, disconnected); session with tunnel; delete fails then succeeds on retry (paused clock); corrupt record (reported, cleared, disconnected).
- Persisted shape (from C1): `crates/helper/src/session_store.rs` `Saved { last_spoke_id, session, pending_deletions }`, `SessionStore::load()`. On macOS b5's `serve` already replays a stored teardown at startup; move that call into recovery if it fits, without changing its behaviour.
