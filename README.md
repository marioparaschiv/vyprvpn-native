# VyprLite

A minimal VyprVPN client: an unprivileged tray app (`VyprLite`) and a privileged helper (`vyprlite-helper`).

## Build

```sh
cargo build --workspace
```

Binaries land in `target/debug/`. Run the tests with `cargo test --workspace`.

## Build a distributable

```sh
cargo xtask dist
```

This builds the tray app and helper in release and writes `dist/` with `VyprLite.exe`, `vyprlite-helper.exe` and the official `wireguard.dll` (WireGuardNT 1.1, amd64). The DLL comes from the WireGuardNT zip on download.wireguard.com, cached under `target/wireguard-nt/`. The build fails unless the zip matches the pinned SHA256 and the DLL carries a valid Authenticode signature from WireGuard LLC. Never commit the DLL.

## Run unelevated for development

```sh
target/debug/vyprlite-helper --console
target/debug/VyprLite --console
```

Both use a pipe only the current user can reach, so they never touch an installed service. The console helper exits 5 s after the last tray app disconnects.

## Install

From an elevated prompt, as the user who will use VyprLite, copy the folder `cargo xtask dist` wrote to `C:\Program Files\VyprLite` and install from there:

```bat
xcopy /e /i dist "C:\Program Files\VyprLite"
cd /d "C:\Program Files\VyprLite"
vyprlite-helper.exe install
```

The service runs `vyprlite-helper.exe` and loads `wireguard.dll` as SYSTEM, so install refuses a folder that anyone but SYSTEM, Administrators and TrustedInstaller can change, including `dist\` itself. It also refuses when anyone else could rename a folder above it.

This registers the `VyprLiteHelper` service (manual start, LocalSystem), lets you start it and query it without elevation, and starts `VyprLite.exe` unelevated at your next login. The tray app starts the service itself, and the service stops 5 s after the tray app exits unless a tunnel is up. Run it again after moving the folder. The service logs to `helper.log`, records panics in `crash.log` and keeps the encrypted login in `credentials.bin` under `%SystemRoot%\System32\config\systemprofile\AppData\Local\VyprLite`.

### macOS

From your own account, in the folder holding `vyprlite-helper` and `VyprLite` (e.g. `target/release` after `cargo build --release`):

```sh
sudo ./vyprlite-helper install
```

This copies the helper to `/Library/PrivilegedHelperTools/com.vyprlite.helper` and the tray app to `/usr/local/libexec/vyprlite/VyprLite`, both root-owned so no other account can replace what root runs. It loads the `com.vyprlite.helper` launch daemon, which runs as root only on demand: the tray app touches `/var/db/vyprlite.wake` (writable only by you) to start it, and it exits 5 s after the tray app does unless a tunnel is up. Only root and you can connect to its socket, `/var/run/vyprlite.sock`. A launch agent in `~/Library/LaunchAgents` starts the tray app at your next login. The helper logs to `helper.log`, records panics in `crash.log` and keeps the login in `credentials.json` (mode 0600) under `/Library/Application Support/VyprLite`, which only root can open.

## Uninstall

From an elevated prompt:

```bat
vyprlite-helper.exe uninstall
```

This disconnects the VPN, removes the service and the login autostart entry, and deletes that data directory with the stored login. Both commands are safe to run twice.

### macOS

```sh
sudo /Library/PrivilegedHelperTools/com.vyprlite.helper uninstall
```

This disconnects the VPN, unloads the launch daemon, and removes both launch plists, the installed binaries, the wake file, the socket and `/Library/Application Support/VyprLite` with the stored login. Both commands are safe to run twice.
