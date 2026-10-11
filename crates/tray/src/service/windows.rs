use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows_service::service::{Service, ServiceAccess};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::{ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_DOES_NOT_EXIST};

use super::InstallError;
use crate::app_dir::MissingEnv;

pub const HELPER_FILE: &str = "vyprvpn-helper.exe";

const HELPER_VAR: &str = "VYPRVPN_HELPER";

/// The elevated helper runs hidden, so only its exit code says why it failed. The helper path goes
/// through the environment so it is never parsed as PowerShell.
const INSTALL_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
$install = Start-Process -FilePath $env:VYPRVPN_HELPER -ArgumentList 'install' -Verb RunAs -WindowStyle Hidden -Wait -PassThru
if ($install.ExitCode) {
    [Console]::Error.WriteLine("vyprvpn-helper install exited with $($install.ExitCode); run it from an administrator prompt to see why")
    exit 1
}"#;

#[derive(Debug, thiserror::Error)]
#[error("{action} failed: {}", ipc::service_error_cause(.source))]
pub struct ServiceError {
    action: &'static str,
    source: windows_service::Error,
}

fn failed(action: &'static str) -> impl Fn(windows_service::Error) -> ServiceError {
    move |source| ServiceError { action, source }
}

/// Install grants the installing user start and query rights, so no elevation is needed.
fn open(access: ServiceAccess) -> Result<Service, ServiceError> {
    ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(failed("connecting to the service control manager"))?
        .open_service(ipc::SERVICE_NAME, access)
        .map_err(failed("opening the helper service"))
}

pub fn start() -> Result<(), ServiceError> {
    let service = open(ServiceAccess::START | ServiceAccess::QUERY_STATUS)?;

    match service.start::<&OsStr>(&[]) {
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(ERROR_SERVICE_ALREADY_RUNNING as i32) =>
        {
            Ok(())
        }
        result => result.map_err(failed("starting the helper service")),
    }
}

/// The helper service's process, or `None` while it is not running.
pub fn process_id() -> Result<Option<u32>, ServiceError> {
    let status = open(ServiceAccess::QUERY_STATUS)?
        .query_status()
        .map_err(failed("querying the helper service"))?;

    Ok(status.process_id)
}

pub fn installed() -> Result<bool, ServiceError> {
    match open(ServiceAccess::QUERY_STATUS) {
        Ok(_) => Ok(true),
        Err(ServiceError {
            source: windows_service::Error::Winapi(error),
            ..
        }) if error.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST as i32) => Ok(false),
        Err(error) => Err(error),
    }
}

/// `Start-Process -Verb RunAs` raises the UAC prompt.
pub fn elevated_install(helper: &Path) -> Result<Command, InstallError> {
    // The full path, since a PATH entry the user can write could otherwise stand in for PowerShell
    // behind the UAC prompt.
    let powershell = PathBuf::from(env::var_os("SystemRoot").ok_or(MissingEnv("SystemRoot"))?)
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let mut command = Command::new(powershell);
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", INSTALL_SCRIPT])
        .env(HELPER_VAR, helper);

    Ok(command)
}
