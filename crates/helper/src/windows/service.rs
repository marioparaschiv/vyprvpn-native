use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::Notify;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

use ipc::SERVICE_NAME;

use super::{HelperError, Sid};

/// The service entry point takes no context, so the owner parsed from the command line and the data
/// directory wait here.
static CONTEXT: OnceLock<(Sid, PathBuf)> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

/// Hands the process to the service control manager; returns once the service has stopped.
pub fn run(owner: Sid, data_dir: PathBuf) -> Result<(), HelperError> {
    CONTEXT.get_or_init(|| (owner, data_dir));
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;

    Ok(())
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(error) = run_service() {
        log::error!("{error}");
    }
}

fn run_service() -> Result<(), HelperError> {
    let stop_requested = Arc::new(Notify::new());
    let status_handle = service_control_handler::register(SERVICE_NAME, {
        let stop_requested = stop_requested.clone();
        move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                stop_requested.notify_one();
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    })?;

    status_handle.set_service_status(service_status(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        ServiceExitCode::Win32(0),
    ))?;

    let (owner, data_dir) = CONTEXT
        .get()
        .expect("run() sets CONTEXT before starting the dispatcher");
    let served = super::serve(ipc::SERVICE_PIPE_NAME, owner, data_dir, async {
        stop_requested.notified().await;
        Ok(())
    });
    let exit_code = match &served {
        Ok(()) => ServiceExitCode::Win32(0),
        Err(error) => {
            log::error!("{error}");
            ServiceExitCode::ServiceSpecific(1)
        }
    };

    status_handle.set_service_status(service_status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        exit_code,
    ))?;

    Ok(())
}

fn service_status(
    state: ServiceState,
    controls_accepted: ServiceControlAccept,
    exit_code: ServiceExitCode,
) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted,
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::ZERO,
        process_id: None,
    }
}
