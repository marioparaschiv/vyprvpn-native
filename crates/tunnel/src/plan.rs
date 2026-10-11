use std::net::{Ipv4Addr, SocketAddrV4};

use ipnet::Ipv4Net;

use crate::config::TunnelConfig;
use crate::keys::{PresharedKey, PrivateKey, PublicKey};

/// Everything to apply to bring the tunnel up. The peer's allowed IPs double as the routes
/// through the tunnel. On Windows no route keeps the endpoint off the tunnel: WireGuardNT already
/// skips its own adapter's routes when it resolves the endpoint. Userspace WireGuard on macOS
/// does not, so that backend adds one.
#[derive(Debug)]
pub struct AdapterPlan {
    pub private_key: PrivateKey,
    pub address: Ipv4Net,
    pub dns: Vec<Ipv4Addr>,
    pub peer: PeerPlan,
}

#[derive(Debug)]
pub struct PeerPlan {
    pub public_key: PublicKey,
    pub preshared_key: PresharedKey,
    pub endpoint: SocketAddrV4,
    pub allowed_ips: [Ipv4Net; 2],
}

impl AdapterPlan {
    /// `endpoint` is the resolved form of `config.peer.endpoint`.
    pub fn new(config: TunnelConfig, endpoint: SocketAddrV4) -> Self {
        let TunnelConfig {
            private_key,
            address,
            dns,
            peer,
        } = config;

        Self {
            private_key,
            address,
            dns,
            peer: PeerPlan {
                public_key: peer.public_key,
                preshared_key: peer.preshared_key,
                endpoint,
                allowed_ips: peer.allowed_ips,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{PEER_PUBLIC, PRESHARED, realistic_plan as plan};

    const ENDPOINT: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 10), 51820);

    #[test]
    fn interface_gets_key_address_and_dns() {
        let plan = plan();

        assert_eq!(plan.private_key.as_bytes(), &[7; 32]);
        assert_eq!(plan.address.to_string(), "10.21.0.7/32");
        assert_eq!(
            plan.dns,
            [Ipv4Addr::new(10, 21, 0, 1), Ipv4Addr::new(10, 21, 0, 2)]
        );
    }

    #[test]
    fn peer_carries_keys_and_resolved_endpoint() {
        let peer = plan().peer;

        assert_eq!(peer.public_key.to_string(), PEER_PUBLIC);
        assert_eq!(peer.preshared_key.to_base64().as_str(), PRESHARED);
        assert_eq!(peer.endpoint, ENDPOINT);
    }

    #[test]
    fn routes_are_exactly_the_split_default_route() {
        let routes = plan().peer.allowed_ips.map(|net| net.to_string());

        assert_eq!(routes, ["0.0.0.0/1", "128.0.0.0/1"]);
    }

    #[test]
    fn debug_output_hides_secrets() {
        let printed = format!("{:?}", plan());

        assert!(printed.contains("PrivateKey(<redacted>)"), "{printed}");
        assert!(printed.contains("PresharedKey(<redacted>)"), "{printed}");
        assert!(printed.contains(PEER_PUBLIC), "{printed}");
    }
}
