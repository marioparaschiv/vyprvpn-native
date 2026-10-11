# VyprLite: all-Rust GPUI tray app + privileged helper

## Goal

Replace the official VyprVPN client (constant 5-10% CPU) with a minimal, low-idle-cost client that:

- lists VyprVPN locations and makes picking one fast (native menus plus a searchable popover),
- logs in and out,
- connects and disconnects over WireGuard,
- runs on Windows now and macOS next.

## Architecture

Two processes:

- **Tray app (Rust + GPUI)**: unprivileged. Native tray icon and right-click menu (`tray-icon` + `muda` crates); GPUI windows for login and the location popover, built with `gpui-kit` (https://gpui-kit.com, its components and theme) on GPUI Fast (its `gpui-fast` feature, https://github.com/longbridge/gpui-fast). Holds no secrets and does no networking to Vypr.
- **Helper (Rust)**: privileged. On Windows a service running as SYSTEM (`windows-service` crate); on macOS a launchd daemon running as root. Owns all Vypr API calls, credential storage, WireGuard keys, and the tunnel. The helper owns the WireGuard adapter directly, so there is no separate tunnel service.

They talk over a local IPC channel:

- Windows: named pipe. macOS: Unix domain socket.
- Protocol: newline-delimited JSON. Requests from the tray app (e.g. status, locations, login, logout, connect, disconnect) get a response; the helper also pushes status events (connecting / connected / disconnected / error) to every connected client.
- Access control: only SYSTEM/root and the user who installed VyprLite may connect. Do not open it to all local users, since the helper controls the network.

### Helper lifecycle

The helper runs only when needed. Nothing runs at boot.

- Windows: the service is manual-start. Install grants the installing user start and query rights on the service, so the unelevated tray app starts it through the Service Control Manager when it launches or finds the helper gone. macOS: launchd socket activation needs unsafe code, so the daemon listens on its Unix socket itself and launchd starts it on demand (WatchPaths) when the tray touches an owner-only wake file created at install.
- The helper exits by itself when no tray app is connected and no tunnel is up. While a tunnel is up it keeps running, even with the tray app closed.
- Startup is fast: the helper listens on the IPC endpoint first; state reconciliation, API calls, and loading `wireguard.dll` happen lazily, after that or on first use.

The helper is the single source of truth for state. The tray app renders whatever the helper reports and must cope with the helper restarting.

## References (read-only)

Never modify, copy wholesale, or build these. Read them for behaviour and payload shapes.

- Go prototype `C:\Users\eternal\vypr-lite\`:
  - `api.go`: headers, the agent digest, endpoint shapes
  - `vpn.go`: config contents, connect/disconnect order
  - `platform_windows.go`: the data-dir ACL, ephemeral-service and credential handling
  - `vpn_windows_test.go`: test payload shapes
- Decompiled VyprVPN `C:\Users\eternal\vypr-decomp\`:
  - `GoldenFrogVPN\GoldenFrogVPN\WireGuardVPNConnector.cs`: connect/disconnect and config
  - `VyprVPNService\VyprVPNService\ServiceApiCaller.cs`
- The official wireguard-windows Go source, for how it configures the adapter (address, routes, DNS, MTU, default-route binding): `C:\Users\eternal\go\pkg\mod\golang.zx2c4.com\wireguard\windows@v1.1.1\tunnel\` (`addressconfig.go`, `winipcfg\`, `interfacewatcher.go`).
- Official signed `wireguard.dll` (WireGuardNT 1.1, amd64) at `C:\Users\eternal\vypr-lite-downloads\wgnt\wireguard-nt\bin\amd64\wireguard.dll`; zip SHA256 `dceb30a9bc4be48cce0f74160fc88a585a2c2627366e8f846fc6658f9038dace`; signer: WireGuard LLC.

## Vypr API facts (reverse-engineered from VyprVPN 6.0.4)

Base URL `https://api.vyprvpn.com`. Standard headers on every call:

- `Content-Type: application/json`
- `X-GF-PLATFORM: Windows` (or `Mac`), `X-GF-PRODUCT: VyprVPN`
- `X-GF-PLATFORM-VERSION: <OS version>`, `X-GF-PRODUCT-VERSION: 6.0.4`
- `User-Agent: <Platform>/<OS version> VyprVPN/6.0.4.0`

Authenticated calls also send:

- `username`, `password` headers (plain, over TLS)
- `X-GF-Agent: VyprVPN <Platform> 6.0.4.0 (<last 8 hex chars of uppercase MD5("VyprVPN"+username+password)>)`

Endpoints:

| Purpose | Call | Notes |
|---|---|---|
| Validate login / account settings | `GET /settings` (auth) | JSON with `realm`. 403 = bad credentials. |
| Locations | `GET /vyprvpn/locations-unauthenticated` | Header `locale: en_US`. Body `{"locations": [...]}`; each has `name`, `country`, `country_code`, `region`, `spoke_id`, `hostname`. ~73 entries. |
| Connect | `POST /vyprvpn/spokes/{spoke_id}/wg/connections` (auth) | Header `X-GF-Realm: <realm>`. Body `{"Server": "<spoke_id>", "ClientKey": "<base64 WG public key>"}`. Response: `ConnectionId`, `Interface.Address`, `Interface.DNS[]`, `Peer.PublicKey`, `Peer.PresharedKey`, `Peer.Endpoint`. |
| Disconnect | `DELETE /vyprvpn/spokes/{spoke_id}/wg/connections/{ConnectionId}` | Header `wireguard-psk: <PresharedKey>`; no username/password. |

Tunnel config: `AllowedIPs = 0.0.0.0/1, 128.0.0.0/1` (split default route, as the official client does, so WireGuard does not engage its own kill switch).

## Decisions

- UI: native tray menu for right-click; GPUI only for windows (login, popover). No webview.
- Left-click on the tray icon opens a GPUI popover anchored to the icon (below the menu-bar icon on macOS; beside the icon on the taskbar's edge on Windows).
- The tray app runs unelevated; elevation happens once, at install, for the helper.
- Credentials live in the helper, encrypted at rest (Windows: DPAPI user scope under SYSTEM, so only SYSTEM can decrypt, stored in the helper's data dir, which is LocalSystem's local app-data dir (`C:\Windows\System32\config\systemprofile\AppData\Local\VyprLite`). It inherits the system profile's SYSTEM+Administrators-only ACL, so no ACL code (and no unsafe) is needed; macOS: System keychain or root-only file).
- Closing the tray app does not drop the VPN; the helper keeps the tunnel up.
- Windows is built and verified first; macOS needs a Mac to build and test.
- No unsafe code: `unsafe_code = "deny"` across the workspace, and OS calls go through maintained crates with safe APIs. The single exception is loading `wireguard.dll` (`wireguard_nt::load_from_path`, which has no safe form), one audited `#[expect(unsafe_code)]` in `crates/tunnel`. Where no safe crate exists, use `netsh` rather than more unsafe.
- Current Rust standards: edition 2024, resolver 3, workspace-level dependencies and lints, latest stable crate versions.

## Glossary

- **Location**: one entry from the locations endpoint; identified by `spoke_id`.
- **Session**: one server-side WireGuard connection (`ConnectionId` + `PresharedKey`), created on connect and deleted on disconnect.
- **Tunnel**: the local WireGuard adapter (owned by the helper) carrying traffic for a session.
- **Helper**: the privileged Rust process. **Tray app**: the unprivileged Rust + GPUI process.
- **Popover**: the borderless GPUI window anchored to the tray icon.
