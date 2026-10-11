use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use ipnet::IpNet;
use wireguard_nt::{Adapter, SetInterface, SetPeer, Wireguard};
use zeroize::Zeroize;

use crate::command::{self, CommandError};
use crate::netsh;
use crate::plan::AdapterPlan;

const DLL_NAME: &str = "wireguard.dll";
/// Names both the adapter and its wireguard-nt pool.
const ADAPTER_NAME: &str = "VyprLite";
/// A stable GUID lets Windows reuse one network profile instead of creating a new one per connect.
const ADAPTER_GUID: u128 = 0x5f1e_6c3a_9b2d_4e71_8a40_c7d3_19b8_e26f;

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
    #[error("SystemRoot is not set, so netsh.exe cannot be located")]
    SystemRootMissing,
    #[error(transparent)]
    Command(#[from] CommandError),
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
    let netsh = netsh_path()?;
    let wireguard = load_wireguard()?;
    let adapter = Adapter::create(&wireguard, ADAPTER_NAME, ADAPTER_NAME, Some(ADAPTER_GUID))
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

    for invocation in netsh::dns_commands(&netsh, ADAPTER_NAME, &plan.dns) {
        command::run(&invocation)?;
    }

    Ok(ActiveTunnel { adapter })
}

impl ActiveTunnel {
    /// Removes the adapter even when taking it down first fails.
    pub fn down(self) -> Result<(), TunnelError> {
        self.adapter.down().map_err(adapter_error("taking down"))
    }
}

/// The absolute path, because a bare `netsh` would be searched for in the helper's own directory
/// first.
fn netsh_path() -> Result<PathBuf, TunnelError> {
    let root = std::env::var_os("SystemRoot").ok_or(TunnelError::SystemRootMissing)?;

    Ok(PathBuf::from(root).join(r"System32\netsh.exe"))
}
