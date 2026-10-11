# Rules for every implementation agent

Repo `C:\Users\eternal\vypr-native-client`, branch `feat/native-client`. Read `parent.md`, your sub-issue in `issues/sub/`, and the existing code you touch before writing. Several agents work in this tree at the same time.

## Ownership
- Edit only the files your sub-issue lists as owned. You may append lines to root `Cargo.toml` `[workspace.dependencies]`; nothing else outside your files.
- If you need a change in a file you don't own, stop and message the team lead with the exact change; don't make it.
- If a workspace-wide check fails only in another agent's files, that's their work in progress: report it, don't fix it.

## Build
- Use your own build dir: `CARGO_TARGET_DIR=target/<your agent name>` on every cargo command.
- Your gate is the verify list in your sub-issue (per package). All of it must pass before you report.
- Never run `cargo fmt` without `--check` on the whole workspace; format only your own files (`rustfmt <file>` or `cargo fmt -p <your package>` when you own the whole package).

## Code
- No unsafe. `unsafe_code = "deny"` is workspace-wide; the only exception is the existing `wireguard.dll` load in `crates/tunnel/src/windows.rs`.
- OS calls go through maintained crates with safe APIs; where none exists, use a command-line tool via `std::process::Command` with checked exit codes, or stop and report.
- Platform gating: OS code behind `#[cfg(target_os = "windows")]` / `#[cfg(target_os = "macos")]` modules, OS-only deps under target-specific dependency tables, no runtime OS checks.
- Current Rust: edition 2024, workspace deps and lints (`[lints] workspace = true`), latest stable crates, thiserror 2, no deprecated APIs.
- Typed errors, never swallowed. rustfmt-clean, clippy `-D warnings`. Comments only for a non-obvious why. No em-dashes.
- Match the existing code's structure and naming; reuse what exists instead of adding parallel mechanisms.
- Layout: every crate lives under `crates/<name>` with the conventional `src/` layout (`lib.rs` or `main.rs`, `tests/` for integration tests). Platform-neutral, GUI-free logic belongs in its own small library crate when it is reused or worth testing without GPUI; binaries stay thin.
- Idle CPU ~0%: event-driven, no polling loops.

## Safety
- Nothing elevated or system-changing: no installing or starting services, no adapters, routes, DNS, registry or firewall changes, no `sudo`.
- Never send input to the desktop. Never capture the screen or a screen region. If your sub-issue asks for screenshots, capture only your own window by its handle (e.g. `PrintWindow`) and check the image before keeping it.
- No live API calls that need the user's credentials. The user's stored login in `%LOCALAPPDATA%\VyprVPN\credentials.bin` must not be read, decrypted, used or deleted. Run dev processes against a throwaway data dir.
- Tests deterministic: no real sleeps, use paused or fake clocks; mock HTTP with a local server.
- Never use `sleep` to wait in your shell. Kill every process you start.
- Don't commit.

## Report
Short. Your sub-issue's criteria, each met or not with evidence; the verify commands and their results; anything another agent or the user must know.
