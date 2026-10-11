use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use windows_service::service::{
    Service, ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType, ServiceState,
    ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::{
    ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_MARKED_FOR_DELETE, ERROR_SERVICE_NOT_ACTIVE,
};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};

use super::install_dir::{self, InstallDirError};
use super::security::{CurrentSidError, Sid};
use crate::disconnect::{DisconnectError, disconnect_blocking};

const SERVICE_DISPLAY_NAME: &str = "VyprVPN Helper";
const HELPER_EXE: &str = "vyprvpn-helper.exe";
/// The tunnel loads it from the helper's folder.
const WIREGUARD_DLL: &str = "wireguard.dll";
const TRAY_EXE: &str = "VyprVPN.exe";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const AUTOSTART_VALUE: &str = "VyprVPN";
const APP_ID_KEYS: &str = r"Software\Classes\AppUserModelId";
const APP_DISPLAY_NAME: &str = "VyprVPN";
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
const STOP_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("access denied: run this from an administrator command prompt")]
    NotElevated,
    #[error(transparent)]
    CurrentUser(#[from] CurrentSidError),
    #[error("finding the helper's own directory failed: {0}")]
    ExeDir(io::Error),
    #[error(transparent)]
    InstallDir(#[from] InstallDirError),
    #[error("environment variable SystemRoot is not set")]
    MissingSystemRoot,
    #[error("{action} failed: {}", ipc::service_error_cause(.source))]
    Service {
        action: &'static str,
        source: windows_service::Error,
    },
    #[error("running sc.exe sdset failed: {0}")]
    SecurityCommand(io::Error),
    #[error("sc.exe sdset failed ({status}): {output}")]
    SecurityRejected { status: String, output: String },
    #[error("the helper service did not stop within {} s", STOP_TIMEOUT.as_secs())]
    StopTimeout,
    #[error("{action} the autostart entry failed: {source}")]
    Autostart {
        action: &'static str,
        source: io::Error,
    },
    #[error("{action} the notification app id failed: {source}")]
    AppId {
        action: &'static str,
        source: io::Error,
    },
    #[error(transparent)]
    Disconnect(#[from] DisconnectError),
    #[error("deleting {path} failed: {source}")]
    DataDir { path: PathBuf, source: io::Error },
}

fn failed(action: &'static str) -> impl Fn(windows_service::Error) -> InstallError {
    move |source| InstallError::Service { action, source }
}

fn is_os_error(error: &windows_service::Error, code: u32) -> bool {
    matches!(error, windows_service::Error::Winapi(os_error) if os_error.raw_os_error() == Some(code as i32))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceConfig {
    executable: PathBuf,
    arguments: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    CreateService(ServiceConfig),
    UpdateService(ServiceConfig),
    SetServiceSecurity {
        sddl: String,
    },
    SetAutostart {
        command: String,
    },
    /// Lets the tray app's notifications show VyprVPN, not the process that launched it, as source.
    RegisterAppId {
        key: String,
        display_name: &'static str,
    },
    Disconnect,
    /// Deletes the service before stopping it, so a running tray app cannot start it again in between.
    RemoveService,
    DeleteAutostart,
    DeleteAppId {
        key: String,
    },
    DeleteDataDir(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelperService {
    Absent,
    Stopped,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Found {
    service: HelperService,
    autostart: bool,
    app_id: bool,
    data_dir: bool,
}

fn install_plan(exe_dir: &Path, owner: &Sid, service: HelperService) -> Vec<Step> {
    let config = ServiceConfig {
        executable: exe_dir.join(HELPER_EXE),
        arguments: ["--service", "--owner-sid", &owner.to_string()]
            .map(OsString::from)
            .into(),
    };
    let register = match service {
        HelperService::Absent => Step::CreateService(config),
        HelperService::Stopped | HelperService::Running => Step::UpdateService(config),
    };

    vec![
        register,
        Step::SetServiceSecurity {
            sddl: service_sddl(owner),
        },
        Step::SetAutostart {
            command: format!("\"{}\"", exe_dir.join(TRAY_EXE).display()),
        },
        Step::RegisterAppId {
            key: app_id_key(),
            display_name: APP_DISPLAY_NAME,
        },
    ]
}

fn uninstall_plan(data_dir: &Path, found: Found) -> Vec<Step> {
    let mut plan = Vec::new();

    if found.service == HelperService::Running {
        plan.push(Step::Disconnect);
    }
    if found.service != HelperService::Absent {
        plan.push(Step::RemoveService);
    }
    if found.autostart {
        plan.push(Step::DeleteAutostart);
    }
    if found.app_id {
        plan.push(Step::DeleteAppId { key: app_id_key() });
    }
    if found.data_dir {
        plan.push(Step::DeleteDataDir(data_dir.to_owned()));
    }

    plan
}

fn app_id_key() -> String {
    format!(r"{APP_ID_KEYS}\{}", ipc::APP_USER_MODEL_ID)
}

/// Windows' default service DACL, plus start (RP) and query status (LC) for the installing user so
/// the unelevated tray app can start the helper.
fn service_sddl(owner: &Sid) -> String {
    format!(
        "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)(A;;RPLC;;;{owner})"
    )
}

pub fn install() -> Result<(), InstallError> {
    let manager = elevated_service_manager()?;
    let owner = Sid::current()?;
    let exe_dir = crate::exe_dir().map_err(InstallError::ExeDir)?;
    // The service runs the helper, and it loads the DLL, as SYSTEM.
    install_dir::check(&system_root()?, &exe_dir, &[HELPER_EXE, WIREGUARD_DLL])?;

    let plan = install_plan(&exe_dir, &owner, find_service(&manager)?);
    execute(&manager, &plan)?;
    println!("VyprVPN is installed and starts at your next login.");

    Ok(())
}

pub fn uninstall() -> Result<(), InstallError> {
    let manager = elevated_service_manager()?;
    let data_dir = service_data_dir()?;
    let found = Found {
        service: find_service(&manager)?,
        autostart: autostart_exists()?,
        app_id: app_id_exists(&app_id_key())?,
        data_dir: data_dir
            .try_exists()
            .map_err(|source| InstallError::DataDir {
                path: data_dir.clone(),
                source,
            })?,
    };

    execute(&manager, &uninstall_plan(&data_dir, found))?;
    println!("VyprVPN is uninstalled.");

    Ok(())
}

/// Creating services needs an elevated token, so this fails early when the user forgot to elevate.
fn elevated_service_manager() -> Result<ServiceManager, InstallError> {
    ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|error| match &error {
        windows_service::Error::Winapi(os_error)
            if os_error.kind() == io::ErrorKind::PermissionDenied =>
        {
            InstallError::NotElevated
        }
        _ => failed("connecting to the service control manager")(error),
    })
}

fn system_root() -> Result<PathBuf, InstallError> {
    env::var_os("SystemRoot")
        .map(PathBuf::from)
        .ok_or(InstallError::MissingSystemRoot)
}

/// LocalSystem's `%LOCALAPPDATA%\VyprVPN`, where the service keeps its log and credentials.
fn service_data_dir() -> Result<PathBuf, InstallError> {
    Ok(system_root()?.join(r"System32\config\systemprofile\AppData\Local\VyprVPN"))
}

fn find_service(manager: &ServiceManager) -> Result<HelperService, InstallError> {
    let service = match manager.open_service(ipc::SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
        Err(error) if is_os_error(&error, ERROR_SERVICE_DOES_NOT_EXIST) => {
            return Ok(HelperService::Absent);
        }
        result => result.map_err(failed("opening the helper service"))?,
    };
    let status = service
        .query_status()
        .map_err(failed("querying the helper service"))?;

    Ok(match status.current_state {
        ServiceState::Stopped => HelperService::Stopped,
        _ => HelperService::Running,
    })
}

fn autostart_exists() -> Result<bool, InstallError> {
    let value = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_QUERY_VALUE)
        .and_then(|key| key.get_raw_value(AUTOSTART_VALUE));

    match value {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(InstallError::Autostart {
            action: "reading",
            source,
        }),
    }
}

fn app_id_exists(key: &str) -> Result<bool, InstallError> {
    match RegKey::predef(HKEY_CURRENT_USER).open_subkey(key) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(InstallError::AppId {
            action: "reading",
            source,
        }),
    }
}

fn execute(manager: &ServiceManager, plan: &[Step]) -> Result<(), InstallError> {
    for step in plan {
        match step {
            Step::CreateService(config) => {
                manager
                    .create_service(&service_info(config), ServiceAccess::QUERY_STATUS)
                    .map_err(failed("registering the helper service"))?;
            }
            Step::UpdateService(config) => manager
                .open_service(ipc::SERVICE_NAME, ServiceAccess::CHANGE_CONFIG)
                .and_then(|service| service.change_config(&service_info(config)))
                .map_err(failed("updating the helper service"))?,
            Step::SetServiceSecurity { sddl } => set_service_security(sddl)?,
            Step::SetAutostart { command } => RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(RUN_KEY)
                .and_then(|(key, _)| key.set_value(AUTOSTART_VALUE, command))
                .map_err(|source| InstallError::Autostart {
                    action: "writing",
                    source,
                })?,
            Step::RegisterAppId { key, display_name } => RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(key)
                .and_then(|(key, _)| key.set_value("DisplayName", display_name))
                .map_err(|source| InstallError::AppId {
                    action: "writing",
                    source,
                })?,
            Step::Disconnect => disconnect_blocking(ipc::SERVICE_PIPE_NAME)?,
            Step::RemoveService => remove_service(manager)?,
            Step::DeleteAutostart => delete_autostart()?,
            Step::DeleteAppId { key } => {
                match RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(key) {
                    Err(source) if source.kind() != io::ErrorKind::NotFound => {
                        return Err(InstallError::AppId {
                            action: "deleting",
                            source,
                        });
                    }
                    _ => {}
                }
            }
            Step::DeleteDataDir(path) => match fs::remove_dir_all(path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => {
                    return Err(InstallError::DataDir {
                        path: path.clone(),
                        source: error,
                    });
                }
                _ => {}
            },
        }
    }

    Ok(())
}

fn service_info(config: &ServiceConfig) -> ServiceInfo {
    ServiceInfo {
        name: ipc::SERVICE_NAME.into(),
        display_name: SERVICE_DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::OnDemand,
        error_control: ServiceErrorControl::Normal,
        executable_path: config.executable.clone(),
        launch_arguments: config.arguments.clone(),
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    }
}

/// `windows-service` has no safe way to set a service's security descriptor.
fn set_service_security(sddl: &str) -> Result<(), InstallError> {
    let output = Command::new(system_root()?.join(r"System32\sc.exe"))
        .args(["sdset", ipc::SERVICE_NAME, sddl])
        .output()
        .map_err(InstallError::SecurityCommand)?;

    if !output.status.success() {
        return Err(InstallError::SecurityRejected {
            status: output.status.to_string(),
            output: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        });
    }

    Ok(())
}

fn remove_service(manager: &ServiceManager) -> Result<(), InstallError> {
    let service = manager
        .open_service(
            ipc::SERVICE_NAME,
            ServiceAccess::DELETE | ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
        )
        .map_err(failed("opening the helper service"))?;

    match service.delete() {
        Err(error) if !is_os_error(&error, ERROR_SERVICE_MARKED_FOR_DELETE) => {
            return Err(failed("deleting the helper service")(error));
        }
        _ => {}
    }
    match service.stop() {
        Err(error) if !is_os_error(&error, ERROR_SERVICE_NOT_ACTIVE) => {
            return Err(failed("stopping the helper service")(error));
        }
        _ => {}
    }

    wait_until_stopped(&service)
}

/// The data directory can only go once the service has let go of its log file.
fn wait_until_stopped(service: &Service) -> Result<(), InstallError> {
    let deadline = Instant::now() + STOP_TIMEOUT;

    while service
        .query_status()
        .map_err(failed("querying the helper service"))?
        .current_state
        != ServiceState::Stopped
    {
        if Instant::now() >= deadline {
            return Err(InstallError::StopTimeout);
        }
        thread::sleep(STOP_POLL_INTERVAL);
    }

    Ok(())
}

fn delete_autostart() -> Result<(), InstallError> {
    let deleted = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
        .and_then(|key| key.delete_value(AUTOSTART_VALUE));

    match deleted {
        Err(source) if source.kind() != io::ErrorKind::NotFound => Err(InstallError::Autostart {
            action: "deleting",
            source,
        }),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWNER: &str = "S-1-5-21-3623811015-3361044348-30300820-1013";

    fn owner() -> Sid {
        Sid::parse(OWNER).unwrap()
    }

    fn found(service: HelperService, autostart: bool, app_id: bool, data_dir: bool) -> Found {
        Found {
            service,
            autostart,
            app_id,
            data_dir,
        }
    }

    #[test]
    fn install_registers_the_service_sets_its_security_and_autostarts_the_tray() {
        let plan = install_plan(
            Path::new(r"C:\Program Files\VyprVPN"),
            &owner(),
            HelperService::Absent,
        );

        assert_eq!(
            plan,
            [
                Step::CreateService(ServiceConfig {
                    executable: PathBuf::from(r"C:\Program Files\VyprVPN\vyprvpn-helper.exe"),
                    arguments: ["--service", "--owner-sid", OWNER]
                        .map(OsString::from)
                        .into(),
                }),
                Step::SetServiceSecurity {
                    sddl: format!(
                        "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)(A;;RPLC;;;{OWNER})"
                    ),
                },
                Step::SetAutostart {
                    command: r#""C:\Program Files\VyprVPN\VyprVPN.exe""#.into(),
                },
                Step::RegisterAppId {
                    key: r"Software\Classes\AppUserModelId\VyprVPN".into(),
                    display_name: "VyprVPN",
                },
            ]
        );
    }

    #[test]
    fn install_updates_an_existing_service_in_place() {
        for service in [HelperService::Stopped, HelperService::Running] {
            let plan = install_plan(Path::new(r"D:\VyprVPN"), &owner(), service);

            assert_eq!(
                plan[0],
                Step::UpdateService(ServiceConfig {
                    executable: PathBuf::from(r"D:\VyprVPN\vyprvpn-helper.exe"),
                    arguments: ["--service", "--owner-sid", OWNER]
                        .map(OsString::from)
                        .into(),
                }),
                "{service:?}"
            );
            assert_eq!(
                plan[1..],
                install_plan(Path::new(r"D:\VyprVPN"), &owner(), HelperService::Absent)[1..]
            );
        }
    }

    #[test]
    fn uninstall_disconnects_a_running_helper_and_removes_everything() {
        let data_dir = Path::new(r"C:\Windows\System32\config\systemprofile\AppData\Local\VyprVPN");

        assert_eq!(
            uninstall_plan(data_dir, found(HelperService::Running, true, true, true)),
            [
                Step::Disconnect,
                Step::RemoveService,
                Step::DeleteAutostart,
                Step::DeleteAppId {
                    key: r"Software\Classes\AppUserModelId\VyprVPN".into(),
                },
                Step::DeleteDataDir(data_dir.to_owned()),
            ]
        );
    }

    #[test]
    fn uninstall_skips_the_disconnect_when_the_helper_is_stopped() {
        let data_dir = Path::new(r"C:\data");

        assert_eq!(
            uninstall_plan(data_dir, found(HelperService::Stopped, false, false, true)),
            [
                Step::RemoveService,
                Step::DeleteDataDir(data_dir.to_owned())
            ]
        );
    }

    #[test]
    fn uninstall_does_nothing_once_everything_is_gone() {
        assert_eq!(
            uninstall_plan(
                Path::new(r"C:\data"),
                found(HelperService::Absent, false, false, false)
            ),
            []
        );
    }
}
