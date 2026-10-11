use tunnel::{ActiveTunnel, AdapterPlan, TunnelError};

use crate::vpn::Tunnel;

/// The WireGuardNT adapter, driven through the `wireguard.dll` next to the helper.
pub struct WireGuardNt;

impl Tunnel for WireGuardNt {
    type Active = ActiveTunnel;
    type Error = TunnelError;

    /// WireGuardNT removes the adapter, and with it the address, routes and DNS, when the helper
    /// that created it exits, so nothing is left to undo.
    fn recover(&self) -> Result<(), TunnelError> {
        Ok(())
    }

    fn up(&self, plan: &AdapterPlan) -> Result<ActiveTunnel, TunnelError> {
        tunnel::up(plan)
    }

    fn down(&self, active: ActiveTunnel) -> Result<(), TunnelError> {
        active.down()
    }
}
