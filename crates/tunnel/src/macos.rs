use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use boringtun::device::{DeviceConfig, DeviceHandle};

use crate::command::{self, CommandError, Invocation};
use crate::macos_plan::{self, NETWORKSETUP, ParseError, ROUTE, SavedDns, Teardown};
use crate::plan::AdapterPlan;

#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error(transparent)]
    Command(#[from] CommandError),
    #[error("{command} reported an error: {output}")]
    NetworksetupFailed { command: String, output: String },
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error("creating WireGuard interface {interface}")]
    Device {
        interface: String,
        #[source]
        source: boringtun::device::Error,
    },
    #[error("configuring WireGuard interface {interface}")]
    Configure {
        interface: String,
        #[source]
        source: io::Error,
    },
    #[error("WireGuard interface {interface} rejected its config (errno {errno})")]
    Rejected { interface: String, errno: i32 },
    #[error("tearing down: {}", describe(.0))]
    Teardown(Vec<TunnelError>),
    #[error("bringing the tunnel up failed, and undoing it also failed: {cleanup}")]
    Rollback {
        #[source]
        source: Box<TunnelError>,
        cleanup: Box<TunnelError>,
    },
}

fn describe(errors: &[TunnelError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

/// A tunnel that is up. Dropping it also runs its teardown and closes the utun.
pub struct ActiveTunnel {
    teardown: Teardown,
    torn_down: bool,
    _device: DeviceHandle,
}

/// Creates a utun driven by boringtun and applies `plan`. On any error the changes made so far
/// are undone.
pub fn up(plan: &AdapterPlan) -> Result<ActiveTunnel, TunnelError> {
    let route = macos_plan::parse_default_route(&run(&macos_plan::default_route_query())?)?;
    let dns = if plan.dns.is_empty() {
        None
    } else {
        Some(saved_dns(&route.interface)?)
    };
    let teardown = Teardown {
        endpoint: *plan.peer.endpoint.ip(),
        dns,
    };

    let interface = macos_plan::free_utun(&run(&macos_plan::interface_list())?);
    let device = DeviceHandle::new(&interface, DeviceConfig::default()).map_err(|source| {
        TunnelError::Device {
            interface: interface.clone(),
            source,
        }
    })?;
    configure(&interface, plan)?;

    let applied = macos_plan::up_commands(&interface, plan, route.gateway, teardown.dns.as_ref())
        .iter()
        .try_for_each(|command| run(command).map(drop));
    if let Err(error) = applied {
        return Err(match teardown.run() {
            Ok(()) => error,
            Err(cleanup) => TunnelError::Rollback {
                source: Box::new(error),
                cleanup: Box::new(cleanup),
            },
        });
    }

    Ok(ActiveTunnel {
        teardown,
        torn_down: false,
        _device: device,
    })
}

impl ActiveTunnel {
    /// What undoes this tunnel. Store it right after `up` so a helper restarted after a crash
    /// can still run it.
    pub fn teardown(&self) -> &Teardown {
        &self.teardown
    }

    pub fn down(mut self) -> Result<(), TunnelError> {
        self.torn_down = true;
        self.teardown.run()
    }
}

impl Drop for ActiveTunnel {
    fn drop(&mut self) {
        if self.torn_down {
            return;
        }

        if let Err(error) = self.teardown.run() {
            log::warn!("tearing down the dropped tunnel: {error}");
        }
    }
}

impl Teardown {
    /// Restores DNS and removes the endpoint route. Runs every step even when one fails, and
    /// treats a route that is already gone as removed, so it is safe to replay after a crash.
    pub fn run(&self) -> Result<(), TunnelError> {
        let failures: Vec<TunnelError> = self
            .commands()
            .iter()
            .filter_map(|command| match run(command) {
                Ok(_) => None,
                Err(TunnelError::Command(CommandError::Failed { output, .. }))
                    if command.program == Path::new(ROUTE)
                        && macos_plan::route_was_missing(&output) =>
                {
                    None
                }
                Err(error) => Some(error),
            })
            .collect();

        if failures.is_empty() {
            Ok(())
        } else {
            Err(TunnelError::Teardown(failures))
        }
    }
}

fn saved_dns(device: &str) -> Result<SavedDns, TunnelError> {
    let order = run(&macos_plan::service_order())?;
    let service = macos_plan::service_for_device(&order, device)?;
    let servers = macos_plan::parse_dns_servers(&run(&macos_plan::dns_servers_query(&service))?)?;

    Ok(SavedDns { service, servers })
}

/// Sends the key and peer over boringtun's UAPI socket, which only root can connect to.
fn configure(interface: &str, plan: &AdapterPlan) -> Result<(), TunnelError> {
    let configure_error = |source| TunnelError::Configure {
        interface: interface.to_owned(),
        source,
    };
    let mut stream =
        UnixStream::connect(macos_plan::uapi_socket(interface)).map_err(configure_error)?;
    stream
        .write_all(macos_plan::uapi_set(plan).as_bytes())
        .map_err(configure_error)?;

    let mut reply = String::new();
    stream.read_to_string(&mut reply).map_err(configure_error)?;

    match macos_plan::parse_uapi_reply(&reply)? {
        0 => Ok(()),
        errno => Err(TunnelError::Rejected {
            interface: interface.to_owned(),
            errno,
        }),
    }
}

/// Runs `invocation` and returns its stdout. networksetup can report an error while exiting 0,
/// so its output is checked too.
fn run(invocation: &Invocation) -> Result<String, TunnelError> {
    let output = command::run(invocation)?;

    if invocation.program == Path::new(NETWORKSETUP) && macos_plan::networksetup_failed(&output) {
        return Err(TunnelError::NetworksetupFailed {
            command: invocation.to_string(),
            output: output.trim().to_owned(),
        });
    }

    Ok(output)
}
