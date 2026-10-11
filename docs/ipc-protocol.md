# VyprLite IPC protocol

The tray app and the helper talk over a local stream. The Rust types in `crates/ipc/src/lib.rs` define the protocol; this page describes them.

## Transport

| Platform | Endpoint | Who may connect |
|---|---|---|
| Windows service | named pipe `\\.\pipe\vyprlite` | SYSTEM and the user who installed VyprLite |
| Windows `--console` helper | named pipe `\\.\pipe\vyprlite-console-<current user SID>` | SYSTEM and the current user |
| macOS | Unix socket `/var/run/vyprlite.sock` | root and the user who installed VyprLite |
| macOS `--console` helper | Unix socket `$TMPDIR/vyprlite-console.sock` | root and the current user |

On Windows the pipe's DACL is `D:P(A;;GA;;;SY)(A;;GRGW;;;<owner SID>)`, and remote clients are rejected. The service learns the owner SID from its command line: `vyprlite-helper --service --owner-sid <SID>`.

## Framing

Each message is one compact JSON object followed by `\n`. A client may send requests at any time; the helper writes responses and events to the same stream in the order they happen. The helper closes the connection if it receives a line it cannot parse.

## Requests (tray app to helper)

Every request has a client-chosen `id` and a `method`.

| Method | Fields | Result |
|---|---|---|
| `status` | none | `{"status": <Status>}` |
| `account` | none | `{"account": <Account>}` |
| `locations` | none | `{"locations": [<Location>, ...]}` |
| `login` | `username`, `password` | `{"account": <Account>}` once VyprVPN accepted the login and the helper stored it |
| `logout` | none | `{"account": <Account>}` once the stored login is deleted |
| `connect` | `spoke_id` | `{"status": <Status>}`, the status the request led to |
| `disconnect` | none | `{"status": <Status>}`, the status the request led to |

```json
{"id":1,"method":"status"}
{"id":2,"method":"locations"}
{"id":3,"method":"login","username":"alice","password":"correct horse"}
{"id":8,"method":"connect","spoke_id":"de1"}
{"id":9,"method":"disconnect"}
```

`login` is the only message that carries the password; no response or event ever does. A rejected login fails with the error `invalid username or password`.

The helper fetches the locations from VyprVPN on the first `locations` request and answers every later one from memory. A failed fetch is not cached, so the next request tries again.

## Messages from the helper

Every message has a `type`.

### `response`

Answers the request with the same `id`, carrying either `result` or `error` (a human-readable message).

```json
{"type":"response","id":1,"result":{"status":{"state":"disconnected","last_spoke_id":null,"error":null}}}
{"type":"response","id":7,"error":"not logged in"}
```

### `status`

Pushed to every connected client whenever the helper's status changes. Its fields are a `Status`.

```json
{"type":"status","state":"connecting","spoke_id":"de1"}
```

### `account`

Pushed to every connected client whenever someone logs in or out. Its fields are an `Account`.

```json
{"type":"account","state":"logged_in","username":"alice"}
```

## Status

| `state` | Fields | Meaning |
|---|---|---|
| `disconnected` | `last_spoke_id`, `error` | No tunnel is up. `last_spoke_id` is the location last connected or attempted, or `null`. `error` is why the last attempt or connection failed, or `null`; it is cleared on the next `connect`. |
| `connecting` | `spoke_id` | Setting up a session and tunnel to that location. |
| `connected` | `spoke_id`, `tunnel_ip`, `connected_at` | The tunnel is up. `tunnel_ip` is the adapter's address (e.g. `10.1.2.3`); `connected_at` is when it came up, in Unix seconds. |
| `disconnecting` | `spoke_id` | Tearing down the tunnel and session. |

```json
{"state":"disconnected","last_spoke_id":"de1","error":"handshake timed out"}
{"state":"connecting","spoke_id":"de1"}
{"state":"connected","spoke_id":"de1","tunnel_ip":"10.1.2.3","connected_at":1760140800}
{"state":"disconnecting","spoke_id":"de1"}
```

## Account

| `state` | Fields | Meaning |
|---|---|---|
| `logged_out` | none | No login is stored. |
| `logged_in` | `username` | A login is stored, encrypted, in the helper's data directory. |

## Location

One VyprVPN location: `name` (e.g. `Australia - Perth`), `country`, `country_code` (three letters, e.g. `AUS`), `region`, `spoke_id` (its identifier, e.g. `au3`) and `hostname`.

## Connection lifecycle

Before sending anything, the tray app checks who serves the pipe, since the helper is usually not running and any local process could create the pipe first. The server's process id (`GetNamedPipeServerProcessId`) must be the `VyprLiteHelper` service's process as the Service Control Manager reports it, or, for a `--console` helper, a process of the current user. Otherwise it shows "Helper not trusted" and sends nothing.

The tray app connects, sends `status` and `account`, then follows the events. It sends `locations` until it has the list, retrying a failed fetch after 1 s, doubling up to 30 s. When the connection drops, or the pipe does not exist, it shows "Helper not running", starts the manual-start service `VyprLiteHelper` through the Service Control Manager (not in `--console` mode), and reconnects with a backoff from 250 ms up to 2 s.

The helper listens on the pipe before doing anything else, and exits once no client has been connected and no tunnel has been up for 5 s.

On macOS the tray app checks the socket server's uid instead: root, or for a `--console` helper the current user. When nothing answers on `/var/run/vyprlite.sock`, it touches `/var/db/vyprlite.wake`, which makes launchd start the daemon.
