use std::fmt;
use std::str::FromStr;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use x25519_dalek::StaticSecret;
use zeroize::Zeroizing;

const KEY_LEN: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("key is not valid base64")]
    Base64(#[from] base64::DecodeError),
    #[error("key is {0} bytes, expected {KEY_LEN}")]
    Length(usize),
    #[error("generating a private key")]
    Random(#[source] getrandom::Error),
}

fn decode_key(encoded: &str) -> Result<Zeroizing<[u8; KEY_LEN]>, KeyError> {
    let bytes = Zeroizing::new(STANDARD.decode(encoded)?);
    if bytes.len() != KEY_LEN {
        return Err(KeyError::Length(bytes.len()));
    }

    let mut key = Zeroizing::new([0; KEY_LEN]);
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// A WireGuard private key. Wiped from memory on drop and never printed.
pub struct PrivateKey(StaticSecret);

impl PrivateKey {
    pub fn generate() -> Result<Self, KeyError> {
        let mut bytes = Zeroizing::new([0; KEY_LEN]);
        getrandom::fill(bytes.as_mut()).map_err(KeyError::Random)?;
        Ok(Self(StaticSecret::from(*bytes)))
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(StaticSecret::from(bytes))
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey(x25519_dalek::PublicKey::from(&self.0).to_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        self.0.as_bytes()
    }
}

impl fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateKey(<redacted>)")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PublicKey([u8; KEY_LEN]);

impl PublicKey {
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }
}

impl FromStr for PublicKey {
    type Err = KeyError;

    fn from_str(encoded: &str) -> Result<Self, Self::Err> {
        decode_key(encoded).map(|key| Self(*key))
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&STANDARD.encode(self.0))
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({self})")
    }
}

/// A WireGuard preshared key. It also authorizes deleting the server-side session, so it is
/// wiped on drop and never printed.
#[derive(Clone, PartialEq, Eq)]
pub struct PresharedKey(Zeroizing<[u8; KEY_LEN]>);

impl PresharedKey {
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    pub fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(STANDARD.encode(*self.0))
    }
}

impl FromStr for PresharedKey {
    type Err = KeyError;

    fn from_str(encoded: &str) -> Result<Self, Self::Err> {
        decode_key(encoded).map(Self)
    }
}

impl fmt::Debug for PresharedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PresharedKey(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::PEER_PUBLIC;

    // RFC 7748 section 6.1 test vector (Alice).
    const PRIVATE: [u8; KEY_LEN] = [
        0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2, 0x66,
        0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5, 0x1d, 0xb9,
        0x2c, 0x2a,
    ];

    #[test]
    fn derives_the_rfc_7748_public_key() {
        let public = PrivateKey::from_bytes(PRIVATE).public_key();

        assert_eq!(public.to_string(), PEER_PUBLIC);
    }

    #[test]
    fn public_key_round_trips_through_base64() {
        let public: PublicKey = PEER_PUBLIC.parse().unwrap();

        assert_eq!(public.to_string(), PEER_PUBLIC);
        assert_eq!(public, PrivateKey::from_bytes(PRIVATE).public_key());
    }

    #[test]
    fn generated_keys_differ() {
        let first = PrivateKey::generate().unwrap();
        let second = PrivateKey::generate().unwrap();

        assert_ne!(first.as_bytes(), second.as_bytes());
        assert_ne!(first.public_key(), second.public_key());
    }

    #[test]
    fn secrets_are_redacted_in_debug_output() {
        let private = PrivateKey::from_bytes(PRIVATE);
        let preshared: PresharedKey = PEER_PUBLIC.parse().unwrap();

        assert_eq!(format!("{private:?}"), "PrivateKey(<redacted>)");
        assert_eq!(format!("{preshared:?}"), "PresharedKey(<redacted>)");
    }

    #[test]
    fn preshared_key_round_trips_through_base64() {
        let preshared: PresharedKey = PEER_PUBLIC.parse().unwrap();

        assert_eq!(preshared.to_base64().as_str(), PEER_PUBLIC);
    }

    #[test]
    fn rejects_malformed_keys() {
        assert!(matches!(
            "not base64!".parse::<PublicKey>(),
            Err(KeyError::Base64(_))
        ));
        assert!(matches!(
            "AAAA".parse::<PresharedKey>(),
            Err(KeyError::Length(3))
        ));
    }
}
