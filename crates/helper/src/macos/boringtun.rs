use std::path::Path;

use tunnel::{ActiveTunnel, AdapterPlan, TunnelError};

use super::{TeardownStore, TeardownStoreError};
use crate::vpn::Tunnel;

#[derive(Debug, thiserror::Error)]
pub enum BoringtunError {
    #[error(transparent)]
    Tunnel(#[from] TunnelError),
    #[error(transparent)]
    Store(#[from] TeardownStoreError),
}

/// A userspace boringtun utun. What undoes each tunnel is stored while it is up, so a helper
/// restarted after a crash restores the routes and DNS.
pub struct Boringtun {
    teardowns: TeardownStore,
}

impl Boringtun {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            teardowns: TeardownStore::new(data_dir),
        }
    }
}

impl Tunnel for Boringtun {
    type Active = ActiveTunnel;
    type Error = BoringtunError;

    /// The utun died with the previous helper, but its routes and DNS changes did not.
    fn recover(&self) -> Result<(), BoringtunError> {
        Ok(self.teardowns.replay()?)
    }

    /// A tunnel whose teardown cannot be stored is dropped, which takes it down again.
    fn up(&self, plan: &AdapterPlan) -> Result<ActiveTunnel, BoringtunError> {
        let active = tunnel::up(plan)?;
        self.teardowns.save(active.teardown())?;

        Ok(active)
    }

    /// On failure the stored teardown is kept for the next start to replay.
    fn down(&self, active: ActiveTunnel) -> Result<(), BoringtunError> {
        active.down()?;
        self.teardowns.clear()?;

        Ok(())
    }
}
