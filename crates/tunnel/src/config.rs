use std::fmt;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs};
use std::str::FromStr;

use ipnet::Ipv4Net;
use serde::Deserialize;

use crate::keys::{KeyError, PresharedKey, PrivateKey, PublicKey};

/// Split default route, as the official client does, so WireGuard does not engage its own kill
/// switch.
pub const ALLOWED_IPS: [Ipv4Net; 2] = [
    Ipv4Net::new_assert(Ipv4Addr::new(0, 0, 0, 0), 1),
    Ipv4Net::new_assert(Ipv4Addr::new(128, 0, 0, 0), 1),
];

/// Body of `POST /vyprvpn/spokes/{spoke_id}/wg/connections`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ConnectResponse {
    pub connection_id: ConnectionId,
    pub interface: InterfaceResponse,
    pub peer: PeerResponse,
}

/// The API has sent this as both a JSON string and a number.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "RawConnectionId")]
pub struct ConnectionId(pub String);

#[derive(Deserialize)]
#[serde(untagged)]
enum RawConnectionId {
    Text(String),
    Number(u64),
}

impl From<RawConnectionId> for ConnectionId {
    fn from(raw: RawConnectionId) -> Self {
        match raw {
            RawConnectionId::Text(text) => Self(text),
            RawConnectionId::Number(number) => Self(number.to_string()),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct InterfaceResponse {
    #[serde(rename = "Address")]
    pub address: String,
    #[serde(rename = "DNS", default)]
    pub dns: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PeerResponse {
    pub public_key: String,
    pub preshared_key: String,
    pub endpoint: String,
}

impl fmt::Debug for PeerResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerResponse")
            .field("public_key", &self.public_key)
            .field("preshared_key", &"<redacted>")
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid interface address {0:?}")]
    Address(String),
    #[error("invalid DNS server {0:?}")]
    Dns(String),
    #[error("invalid peer public key")]
    PublicKey(#[source] KeyError),
    #[error("invalid peer preshared key")]
    PresharedKey(#[source] KeyError),
    #[error(transparent)]
    Endpoint(#[from] EndpointError),
}

#[derive(Debug, thiserror::Error)]
pub enum EndpointError {
    #[error("endpoint {0:?} is not host:port")]
    Format(String),
    #[error("endpoint {0:?} has an invalid port")]
    Port(String),
    #[error("resolving endpoint {endpoint}")]
    Resolve {
        endpoint: Endpoint,
        #[source]
        source: std::io::Error,
    },
    #[error("endpoint {0} has no IPv4 address")]
    NoIpv4(Endpoint),
}

/// The peer's endpoint as the API sent it. A hostname is only looked up by [`Endpoint::resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    /// Blocks on DNS when the host is a name rather than an IPv4 literal.
    pub fn resolve(&self) -> Result<SocketAddrV4, EndpointError> {
        if let Ok(ip) = self.host.parse::<Ipv4Addr>() {
            return Ok(SocketAddrV4::new(ip, self.port));
        }

        let addresses = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|source| EndpointError::Resolve {
                endpoint: self.clone(),
                source,
            })?;

        addresses
            .into_iter()
            .find_map(|address| match address {
                SocketAddr::V4(v4) => Some(v4),
                SocketAddr::V6(_) => None,
            })
            .ok_or_else(|| EndpointError::NoIpv4(self.clone()))
    }
}

impl FromStr for Endpoint {
    type Err = EndpointError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (host, port) = text
            .rsplit_once(':')
            .filter(|(host, _)| !host.is_empty())
            .ok_or_else(|| EndpointError::Format(text.to_owned()))?;
        let port = port
            .parse()
            .ok()
            .filter(|&port| port != 0)
            .ok_or_else(|| EndpointError::Port(text.to_owned()))?;

        Ok(Self {
            host: host.to_owned(),
            port,
        })
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// A validated tunnel config for one session.
#[derive(Debug)]
pub struct TunnelConfig {
    pub private_key: PrivateKey,
    pub address: Ipv4Net,
    pub dns: Vec<Ipv4Addr>,
    pub peer: PeerConfig,
}

#[derive(Debug)]
pub struct PeerConfig {
    pub public_key: PublicKey,
    pub preshared_key: PresharedKey,
    pub endpoint: Endpoint,
    pub allowed_ips: [Ipv4Net; 2],
}

impl TunnelConfig {
    pub fn from_response(
        response: &ConnectResponse,
        private_key: PrivateKey,
    ) -> Result<Self, ConfigError> {
        let peer = &response.peer;
        let dns = response
            .interface
            .dns
            .iter()
            .map(|server| {
                server
                    .trim()
                    .parse()
                    .map_err(|_| ConfigError::Dns(server.clone()))
            })
            .collect::<Result<_, _>>()?;

        Ok(Self {
            private_key,
            address: parse_address(&response.interface.address)?,
            dns,
            peer: PeerConfig {
                public_key: peer.public_key.parse().map_err(ConfigError::PublicKey)?,
                preshared_key: peer
                    .preshared_key
                    .parse()
                    .map_err(ConfigError::PresharedKey)?,
                endpoint: peer.endpoint.parse()?,
                allowed_ips: ALLOWED_IPS,
            },
        })
    }
}

/// wg-quick, which the official client feeds this value to, also accepts a bare address.
fn parse_address(text: &str) -> Result<Ipv4Net, ConfigError> {
    let text = text.trim();

    text.parse()
        .or_else(|_| text.parse::<Ipv4Addr>().map(Ipv4Net::from))
        .map_err(|_| ConfigError::Address(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{PEER_PUBLIC, PRESHARED, realistic_response};

    fn parse(response: &ConnectResponse) -> Result<TunnelConfig, ConfigError> {
        TunnelConfig::from_response(response, PrivateKey::from_bytes([1; 32]))
    }

    #[test]
    fn turns_a_connect_response_into_interface_and_peer_config() {
        let private_key = PrivateKey::from_bytes([7; 32]);

        let config = TunnelConfig::from_response(&realistic_response(), private_key).unwrap();

        assert_eq!(config.private_key.as_bytes(), &[7; 32]);
        assert_eq!(config.address.to_string(), "10.21.0.7/32");
        assert_eq!(
            config.dns,
            [Ipv4Addr::new(10, 21, 0, 1), Ipv4Addr::new(10, 21, 0, 2)]
        );
        assert_eq!(config.peer.public_key.to_string(), PEER_PUBLIC);
        assert_eq!(config.peer.preshared_key.to_base64().as_str(), PRESHARED);
        assert_eq!(
            config.peer.endpoint,
            Endpoint {
                host: "203.0.113.10".into(),
                port: 51820
            }
        );
        assert_eq!(
            config.peer.allowed_ips.map(|net| net.to_string()),
            ["0.0.0.0/1", "128.0.0.0/1"]
        );
    }

    #[test]
    fn connection_id_accepts_numbers_and_strings() {
        let number: ConnectionId = serde_json::from_str("48213977").unwrap();
        let text: ConnectionId = serde_json::from_str(r#""abc-123""#).unwrap();

        assert_eq!(number, ConnectionId("48213977".into()));
        assert_eq!(text, ConnectionId("abc-123".into()));
    }

    #[test]
    fn missing_dns_means_no_servers() {
        let mut response = realistic_response();
        response.interface = serde_json::from_str(r#"{"Address": "10.21.0.7/32"}"#).unwrap();

        assert!(parse(&response).unwrap().dns.is_empty());
    }

    #[test]
    fn bare_address_becomes_a_host_prefix() {
        let mut response = realistic_response();
        response.interface.address = "10.21.0.7".into();

        assert_eq!(
            parse(&response).unwrap().address.to_string(),
            "10.21.0.7/32"
        );
    }

    #[test]
    fn rejects_invalid_fields() {
        type Corrupt = fn(&mut ConnectResponse);
        type Expect = fn(&ConfigError) -> bool;
        let cases: [(Corrupt, Expect); 5] = [
            (
                |r| r.interface.address = "fd00::2/128".into(),
                |e| matches!(e, ConfigError::Address(_)),
            ),
            (
                |r| r.interface.dns.push("dns.example".into()),
                |e| matches!(e, ConfigError::Dns(_)),
            ),
            (
                |r| r.peer.public_key = "short".into(),
                |e| matches!(e, ConfigError::PublicKey(_)),
            ),
            (
                |r| r.peer.preshared_key = "AAAA".into(),
                |e| matches!(e, ConfigError::PresharedKey(KeyError::Length(3))),
            ),
            (
                |r| r.peer.endpoint = "203.0.113.10".into(),
                |e| matches!(e, ConfigError::Endpoint(EndpointError::Format(_))),
            ),
        ];

        for (corrupt, expected) in cases {
            let mut response = realistic_response();
            corrupt(&mut response);

            let error = parse(&response).unwrap_err();

            assert!(expected(&error), "unexpected error: {error:?}");
        }
    }

    #[test]
    fn parses_hostname_endpoints_without_resolving() {
        let endpoint: Endpoint = "de1.vyprvpn.com:51820".parse().unwrap();

        assert_eq!(endpoint.host, "de1.vyprvpn.com");
        assert_eq!(endpoint.port, 51820);
    }

    #[test]
    fn rejects_bad_endpoints() {
        for text in ["host:0", "host:70000", "host:", ":51820", "host"] {
            assert!(text.parse::<Endpoint>().is_err(), "{text} parsed");
        }
    }

    #[test]
    fn resolves_ipv4_literals_without_dns() {
        let endpoint: Endpoint = "203.0.113.10:51820".parse().unwrap();

        assert_eq!(
            endpoint.resolve().unwrap(),
            SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 10), 51820)
        );
    }

    #[test]
    fn debug_output_hides_the_preshared_key() {
        let printed = format!("{:?}", realistic_response());

        assert!(!printed.contains(PRESHARED), "{printed}");
    }
}
