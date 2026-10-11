use std::io;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// The manual-start service the tray app starts on demand.
pub const SERVICE_NAME: &str = "VyprLiteHelper";
pub const SERVICE_PIPE_NAME: &str = r"\\.\pipe\vyprlite";
/// Names the tray app's notifications; install registers it so Windows shows "VyprLite" as their source.
pub const APP_USER_MODEL_ID: &str = "VyprLite";

/// The pipe a `--console` helper serves on, scoped to the user so it never collides with the service.
pub fn console_pipe_name(user_sid: &str) -> String {
    format!(r"\\.\pipe\vyprlite-console-{user_sid}")
}

/// The string SID (`S-1-5-21-...`) of the user this process runs as.
pub fn current_user_sid() -> io::Result<String> {
    process_user_sid(std::process::id())
}

/// The string SID of the user process `process_id` runs as. Fails for processes this user may not
/// inspect, such as SYSTEM's.
pub fn process_user_sid(process_id: u32) -> io::Result<String> {
    let pid = Pid::from_u32(process_id);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing().with_user(UpdateKind::Always),
    );

    let user = system
        .process(pid)
        .and_then(|process| process.user_id())
        .ok_or_else(|| io::Error::other(format!("the user of process {process_id} is unknown")))?;

    Ok(user.to_string())
}

/// `windows_service::Error` only says "IO error in winapi call"; the OS error underneath says why.
pub fn service_error_cause(error: &windows_service::Error) -> String {
    match error {
        windows_service::Error::Winapi(os_error) => os_error.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_user_sid_is_a_user_sid() {
        let sid = current_user_sid().unwrap();

        assert!(sid.starts_with("S-1-5-"), "{sid}");
    }
}
