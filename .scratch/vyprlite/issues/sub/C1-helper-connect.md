# C1: Helper connect, disconnect, switch (helper side of 04)

Owned: `crates/helper/src/` except `windows/install.rs` (B4) and `macos/**` (B5). Starts after B1 (protocol) and B2 (API calls) land.

- Handle `connect { spoke_id }` and `disconnect` from the B1 protocol. Connect: require login, generate a key pair, call the B2 `connect` API, build the `tunnel` plan, resolve the endpoint and bring the tunnel up (`tunnel::up`, in `spawn_blocking`), then push `connected` with the tunnel IP and `connected_at`. Disconnect: tunnel down, then the B2 `disconnect` API. Connecting while connected switches (disconnect, then connect), ending with exactly one session and one tunnel.
- Every step's failure leaves a consistent state: tunnel down, the server session deleted if it was created, then `disconnected` with `error` set. Never stuck in `connecting`.
- Push a `status` event on every transition to every client.
- Persist the active session (spoke_id, ConnectionId, PSK) in the data dir while connected, and the last spoke_id across restarts (`last_spoke_id`); C1 only writes them, D2 uses them for recovery. Never persist the private key.
- Logout also disconnects (ticket 03's deferred item).
- Lifecycle: the helper keeps running while a tunnel is up, even with no client (parent "Helper lifecycle").
- Model connect/disconnect as a state machine over traits for the API and the tunnel, so it's tested with fakes: success, API failure, tunnel failure after a session was created (session deleted), switch, disconnect while connecting, logout while connected.

Verify: `cargo fmt --all --check`; `cargo clippy -p vyprlite-helper --all-targets -- -D warnings`; `cargo test -p vyprlite-helper` with the state-machine cases above and a protocol-level test over the real pipe in console mode using fakes (no real tunnel, no live API).
