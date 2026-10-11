# B1: IPC protocol for connect/disconnect (part of 04, 06)

Owned: `crates/ipc/src/lib.rs` (except the macOS cfg lines B5 adds), `docs/ipc-protocol.md`.

Add exactly this shape (Rust names follow the existing style; the wire format below is the contract other agents code against):
- `Method::Connect { spoke_id: String }` -> `{"id":N,"method":"connect","spoke_id":"de1"}`; `Method::Disconnect` -> `{"id":N,"method":"disconnect"}`. Both reply with the resulting `Status`, or an error.
- `Status` (pushed as an event on every change and returned by `status`):
  - `{"state":"disconnected","last_spoke_id":"de1"|null,"error":"..."|null}`: `error` is the reason the last attempt or connection failed, cleared on the next connect.
  - `{"state":"connecting","spoke_id":"de1"}`
  - `{"state":"connected","spoke_id":"de1","tunnel_ip":"10.1.2.3","connected_at":1760140800}` (unix seconds)
  - `{"state":"disconnecting","spoke_id":"de1"}`
- Update every existing use of `Status::Disconnected` in helper and tray to the new fields with the minimum edit (`last_spoke_id: None, error: None`); that edit in those files is allowed.
- Document all of it in `docs/ipc-protocol.md`.

Verify (all pass): `cargo fmt --all --check`; `cargo clippy -p ipc -p vyprlite-helper -p vyprlite-tray --all-targets -- -D warnings`; `cargo test -p ipc` with a wire-format test for every new message (exact JSON strings above).
