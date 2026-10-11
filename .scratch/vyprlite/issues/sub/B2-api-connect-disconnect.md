# B2: Vypr API connect and disconnect calls (part of 04)

Owned: `crates/vypr-api/**`.

- `connect(credentials, realm, spoke_id, client_public_key_base64) -> Result<ConnectResponse, ApiError>`: `POST /vyprvpn/spokes/{spoke_id}/wg/connections` with the auth headers, `X-GF-Realm`, body `{"Server": spoke_id, "ClientKey": key}` (parent.md "Vypr API facts"). The realm comes from `GET /settings` (reuse what login already fetches; expose it if needed).
- `disconnect(spoke_id, connection_id, preshared_key) -> Result<(), ApiError>`: `DELETE /vyprvpn/spokes/{spoke_id}/wg/connections/{ConnectionId}` with header `wireguard-psk`, no username/password headers.
- Response type: reuse `tunnel::ConnectResponse` (`crates/tunnel/src/config.rs`) if the dependency direction is clean (vypr-api depending on tunnel is fine; tunnel must not depend on vypr-api). Otherwise move nothing and report.
- PSK and password headers marked sensitive; nothing secret in `Debug` or logs.
- References (read-only): `C:\Users\eternal\vypr-lite\api.go`, `vpn.go`, `vpn_windows_test.go`.

Verify: `cargo fmt -p vypr-api --check`; `cargo clippy -p vypr-api --all-targets -- -D warnings`; `cargo test -p vypr-api` with mock-server tests asserting method, path, every header and the body for both calls, plus 401/403 -> `InvalidCredentials`, 5xx -> `Http`, bad JSON -> `Decode`.
