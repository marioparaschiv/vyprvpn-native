//! WireGuard keys, the tunnel config model and the OS tunnel backend.
//!
//! Everything except the OS backend is platform independent: a connect response becomes a
//! [`TunnelConfig`], which together with the resolved endpoint becomes an [`AdapterPlan`] that
//! the backend applies with `up`.

#[cfg(any(target_os = "windows", target_os = "macos", test))]
mod command;
mod config;
#[cfg(test)]
mod fixtures;
mod keys;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
#[cfg_attr(
    not(target_os = "macos"),
    expect(dead_code, reason = "off macOS only its tests use it")
)]
mod macos_plan;
#[cfg(any(target_os = "windows", test))]
mod netsh;
mod plan;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub use command::CommandError;
pub use config::{
    ConfigError, ConnectResponse, ConnectionId, Endpoint, EndpointError, InterfaceResponse,
    PeerConfig, PeerResponse, TunnelConfig,
};
pub use keys::{KeyError, PresharedKey, PrivateKey, PublicKey};
#[cfg(target_os = "macos")]
pub use macos::{ActiveTunnel, TunnelError, up};
#[cfg(target_os = "macos")]
pub use macos_plan::{ParseError, Teardown};
pub use plan::{AdapterPlan, PeerPlan};
#[cfg(target_os = "windows")]
pub use windows::{ActiveTunnel, TunnelError, up};
