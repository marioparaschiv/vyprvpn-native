use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

use ipnet::IpNet;
use wireguard_nt::{Adapter, SetInterface, SetPeer, Wireguard};
use zeroize::Zeroize;

use crate::ADAPTER_NAME;
use crate::netsh;
use crate::plan::AdapterPlan;

const DLL_NAME: &str = "wireguard.dll";
const POOL: &str = "VyprLite";
/// A stable GUID lets Windows reuse one network profile instead of creating a new one per connect.
const ADAPTER_GUID: u128 = 0x5f1e_6c3a_9b2d_4e71_8a40_c7d3_19b8_e26f;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_NOT_FOUND: i32 = 1168;

#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("locating the helper executable")]
    ExePath(#[source] io::Error),
    #[error("{} is missing", .0.display())]
    DllMissing(PathBuf),
    #[error("loading {}", .path.display())]
    DllLoad {
        path: PathBuf,
        #[source]
        source: libloading::Error,
    },
    #[error("{operation} on the WireGuard adapter")]
    Adapter {
        operation: &'static str,
        #[source]
        source: wireguard_nt::Error,
    },
    #[error("running netsh {args}")]
    NetshSpawn {
        args: String,
        #[source]
        source: io::Error,
    },
    #[error("netsh {args} failed ({status}): {output}")]
    Netsh {
        args: String,
        status: std::process::ExitStatus,
        output: String,
    },
}

fn adapter_error(operation: &'static str) -> impl FnOnce(wireguard_nt::Error) -> TunnelError {
    move |source| TunnelError::Adapter { operation, source }
}

/// Loads `wireguard.dll` from the helper executable's directory. Called on connect, never at
/// startup.
fn load_wireguard() -> Result<Wireguard, TunnelError> {
    let exe = std::env::current_exe().map_err(TunnelError::ExePath)?;
    let path = exe.with_file_name(DLL_NAME);
    if !path.is_file() {
        return Err(TunnelError::DllMissing(path));
    }

    load_from_path(&path).map_err(|source| TunnelError::DllLoad { path, source })
}

#[expect(
    unsafe_code,
    reason = "wireguard-nt has no safe way to load wireguard.dll"
)]
fn load_from_path(path: &Path) -> Result<Wireguard, libloading::Error> {
    // SAFETY: `path` is the official WireGuard LLC signed wireguard.dll, checked by hash and
    // signature at build time, in the helper's own install directory, which only administrators
    // can write after install. Its exports match the signatures wireguard-nt binds.
    unsafe { wireguard_nt::load_from_path(path) }
}

/// A tunnel that is up. Dropping it also removes the adapter, its address, routes and DNS.
pub struct ActiveTunnel {
    adapter: Adapter,
}

/// Creates the adapter and applies `plan`. On any error the adapter is removed again.
pub fn up(plan: &AdapterPlan) -> Result<ActiveTunnel, TunnelError> {
    let wireguard = load_wireguard()?;
    let adapter = Adapter::create(&wireguard, POOL, ADAPTER_NAME, Some(ADAPTER_GUID))
        .map_err(adapter_error("creating"))?;

    let mut interface = SetInterface {
        listen_port: None,
        public_key: None,
        private_key: Some(*plan.private_key.as_bytes()),
        peers: vec![SetPeer {
            public_key: Some(*plan.peer.public_key.as_bytes()),
            preshared_key: Some(*plan.peer.preshared_key.as_bytes()),
            keep_alive: None,
            endpoint: SocketAddr::V4(plan.peer.endpoint),
            allowed_ips: plan.peer.allowed_ips.map(IpNet::V4).to_vec(),
        }],
    };
    let configured = adapter.set_config(&interface);
    interface.private_key.zeroize();
    interface.peers[0].preshared_key.zeroize();
    configured.map_err(adapter_error("configuring"))?;

    // Adds the address and a route per allowed IP on the adapter, and sets MTU 1420 and
    // interface metric 0.
    adapter
        .set_default_route(&[IpNet::V4(plan.address)], &interface)
        .map_err(adapter_error("setting the address and routes"))?;
    adapter.up().map_err(adapter_error("bringing up"))?;

    for args in netsh::dns_commands(ADAPTER_NAME, &plan.dns) {
        run_netsh(&args)?;
    }

    Ok(ActiveTunnel { adapter })
}

impl ActiveTunnel {
    /// Removes the adapter even when taking it down first fails.
    pub fn down(self) -> Result<(), TunnelError> {
        self.adapter.down().map_err(adapter_error("taking down"))
    }
}

/// Whether a VyprLite adapter exists. WireGuardNT removes an adapter when the process that
/// created it exits, so one found here belongs to a running helper.
pub fn adapter_exists() -> Result<bool, TunnelError> {
    let wireguard = load_wireguard()?;

    match Adapter::open(&wireguard, ADAPTER_NAME) {
        Ok(_) => Ok(true),
        Err(wireguard_nt::Error::Driver(error))
            if matches!(
                error.raw_os_error(),
                Some(ERROR_FILE_NOT_FOUND | ERROR_NOT_FOUND)
            ) =>
        {
            Ok(false)
        }
        Err(source) => Err(adapter_error("opening")(source)),
    }
}

fn run_netsh(args: &[String]) -> Result<(), TunnelError> {
    let joined = args.join(" ");
    let output = Command::new("netsh")
        .args(args)
        .output()
        .map_err(|source| TunnelError::NetshSpawn {
            args: joined.clone(),
            source,
        })?;

    if output.status.success() {
        return Ok(());
    }

    // netsh reports most errors on stdout.
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));

    Err(TunnelError::Netsh {
        args: joined,
        status: output.status,
        output: text.trim().to_owned(),
    })
}
