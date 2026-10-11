use crate::config::{ConnectResponse, TunnelConfig};
use crate::keys::PrivateKey;
use crate::plan::AdapterPlan;

pub const PEER_PUBLIC: &str = "hSDwCYkwp1R0i33ctD73Wg2/Og0mOBr066SpjqqbTmo=";
pub const PRESHARED: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

/// Shaped like the connect response in the Go prototype's tests.
pub fn realistic_response() -> ConnectResponse {
    let json = format!(
        r#"{{
            "ConnectionId": 48213977,
            "Interface": {{ "Address": "10.21.0.7/32", "DNS": ["10.21.0.1", "10.21.0.2"] }},
            "Peer": {{
                "PublicKey": "{PEER_PUBLIC}",
                "PresharedKey": "{PRESHARED}",
                "Endpoint": "203.0.113.10:51820"
            }}
        }}"#
    );

    serde_json::from_str(&json).unwrap()
}

/// [`realistic_response`] with private key bytes `[7; 32]` and its literal endpoint.
pub fn realistic_plan() -> AdapterPlan {
    let config =
        TunnelConfig::from_response(&realistic_response(), PrivateKey::from_bytes([7; 32]))
            .unwrap();
    let endpoint = config.peer.endpoint.resolve().unwrap();

    AdapterPlan::new(config, endpoint)
}
