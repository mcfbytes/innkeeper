//! Pseudonymous peer labels for public-mode logs (`innkeeperd --public`).

use std::fmt;
use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// HMAC-SHA256 key used to mint peer tokens; random per process, never written down.
#[derive(Clone, Debug)]
pub(crate) struct PeerTokens {
    secret: [u8; 32],
}

impl PeerTokens {
    pub(crate) fn new() -> Self {
        PeerTokens {
            secret: random_secret(),
        }
    }

    pub(crate) fn token(&self, ip: IpAddr) -> PeerToken {
        PeerToken(peer_token(&self.secret, ip))
    }
}

/// First 8 hex chars of HMAC-SHA256(secret, ip). Ports are dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PeerToken([u8; 4]);

impl fmt::Display for PeerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// What a log line prints for a peer: the socket address, or a token in public mode.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PeerLog {
    Addr(SocketAddr),
    Token(PeerToken),
}

impl fmt::Display for PeerLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeerLog::Addr(addr) => write!(f, "{addr}"),
            PeerLog::Token(token) => write!(f, "{token}"),
        }
    }
}

/// HMAC-SHA256 of `ip` keyed by `secret`; the first four digest bytes (eight hex chars).
pub(crate) fn peer_token(secret: &[u8], ip: IpAddr) -> [u8; 4] {
    let mut mac = HmacSha256::new_from_slice(secret)
        .unwrap_or_else(|_| unreachable!("HMAC-SHA256 accepts any key length"));
    match ip {
        IpAddr::V4(addr) => mac.update(&addr.octets()),
        IpAddr::V6(addr) => mac.update(&addr.octets()),
    }
    let digest = mac.finalize().into_bytes();
    let mut token = [0u8; 4];
    if let Some(prefix) = digest.get(..4) {
        token.copy_from_slice(prefix);
    }
    token
}

fn random_secret() -> [u8; 32] {
    let mut secret = [0u8; 32];
    if fill_from_urandom(&mut secret) {
        return secret;
    }
    let mut hasher = Sha256::new();
    hasher.update(std::process::id().to_le_bytes());
    if let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) {
        hasher.update(now.as_nanos().to_le_bytes());
    }
    hasher.update((std::ptr::addr_of!(secret) as usize).to_le_bytes());
    let hash = hasher.finalize();
    if let Some(bytes) = hash.get(..32) {
        secret.copy_from_slice(bytes);
    }
    secret
}

fn fill_from_urandom(secret: &mut [u8; 32]) -> bool {
    let mut file = match std::fs::File::open("/dev/urandom") {
        Ok(file) => file,
        Err(_) => return false,
    };
    file.read_exact(secret).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peer_token_is_stable_for_one_secret_and_never_the_ip() {
        let ip: IpAddr = "203.0.113.50".parse().unwrap();
        let secret_a = [0x11; 32];
        let secret_b = [0x22; 32];
        let a = PeerToken(peer_token(&secret_a, ip)).to_string();
        let again = PeerToken(peer_token(&secret_a, ip)).to_string();
        let other = PeerToken(peer_token(&secret_b, ip)).to_string();
        assert_eq!(a, again);
        assert_ne!(a, other);
        assert_eq!(a.len(), 8);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!a.contains("203"));
        assert!(!a.contains("113"));
        assert!(!a.contains("50"));
        assert_ne!(a, ip.to_string());
        let v6: IpAddr = "2001:db8::1".parse().unwrap();
        let token = PeerToken(peer_token(&secret_a, v6)).to_string();
        assert_ne!(token, v6.to_string());
        assert!(!token.contains("2001"));
        assert!(!token.contains("db8"));
    }
}
