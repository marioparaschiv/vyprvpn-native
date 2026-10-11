use std::ffi::OsStr;

use windows_service::service::{Service, ServiceAccess};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::ERROR_SERVICE_ALREADY_RUNNING;

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
