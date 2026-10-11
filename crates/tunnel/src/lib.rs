//! WireGuard keys, the tunnel config model and the OS tunnel backend.
//!
//! Everything except the OS backend is platform independent: a connect response becomes a
//! [`TunnelConfig`], which together with the resolved endpoint becomes an [`AdapterPlan`] that
//! the backend applies with `up`.

mod config;
#[cfg(test)]
mod fixtures;
mod keys;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod macos_plan;
#[cfg(any(target_os = "windows", test))]
mod netsh;
mod plan;
#[cfg(target_os = "windows")]
mod windows;

pub use config::{
    ALLOWED_IPS, ConfigError, ConnectResponse, ConnectionId, Endpoint, EndpointError,
    InterfaceResponse, PeerConfig, PeerResponse, TunnelConfig,
};
pub use keys::{KeyError, PresharedKey, PrivateKey, PublicKey};
#[cfg(target_os = "macos")]
pub use macos::{ActiveTunnel, TunnelError, up};
#[cfg(target_os = "macos")]
pub use macos_plan::{ParseError, SavedDns, Teardown};
pub use plan::{AdapterPlan, PeerPlan};
#[cfg(target_os = "windows")]
pub use windows::{ActiveTunnel, TunnelError, adapter_exists, up};

/// Name of the WireGuard adapter, also used to find an existing one.
pub const ADAPTER_NAME: &str = "VyprLite";
