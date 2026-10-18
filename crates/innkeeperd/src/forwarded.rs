//! The client's address behind the TLS proxy: the right-most `X-Forwarded-For` entry when the
//! socket peer is loopback (docs/server/hardening.md section 1, privacy.md section 5).

use std::fmt;
use std::net::{IpAddr, SocketAddr};

use hyper::header::HeaderMap;

const FORWARDED_FOR: &str = "x-forwarded-for";

/// Where a line's client address comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Forwarding {
    /// The socket peer is the client: no proxy is trusted (the default outside `--public`).
    #[default]
    Direct,
    /// A loopback peer is the proxy, and the client is its right-most `X-Forwarded-For` entry.
    /// `peer_when_absent` lets a loopback-only development run (`--insecure-dev-keys`) take a
    /// browser on the same machine without a proxy as the client.
    Proxy { peer_when_absent: bool },
}

/// Why no client address could be found; the header's text is never part of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ForwardError {
    /// A loopback peer sent no `X-Forwarded-For`.
    Missing,
    /// The right-most entry is not one IP address.
    Malformed,
}

impl fmt::Display for ForwardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ForwardError::Missing => "the proxy sent no X-Forwarded-For",
            ForwardError::Malformed => "the X-Forwarded-For entry is not an IP address",
        })
    }
}

/// The client's address. One that came from the header has port 0: the proxy does not pass the
/// client's port, and 0 is never a TCP source port, so the sealed record names it as unknown.
pub(crate) fn client_addr(
    headers: &HeaderMap,
    peer: SocketAddr,
    forwarding: Forwarding,
) -> Result<SocketAddr, ForwardError> {
    let Forwarding::Proxy { peer_when_absent } = forwarding else {
        return Ok(peer);
    };
    if !peer.ip().to_canonical().is_loopback() {
        return Ok(peer);
    }
    match rightmost(headers)? {
        Some(ip) => Ok(SocketAddr::new(ip, 0)),
        None if peer_when_absent => Ok(peer),
        None => Err(ForwardError::Missing),
    }
}

/// The last entry of the last header line: the address the proxy itself saw. Entries further left
/// were sent by the client or by proxies nobody here trusts.
fn rightmost(headers: &HeaderMap) -> Result<Option<IpAddr>, ForwardError> {
    let Some(last) = headers.get_all(FORWARDED_FOR).iter().next_back() else {
        return Ok(None);
    };
    let text = last.to_str().map_err(|_| ForwardError::Malformed)?;
    let entry = text.rsplit(',').next().unwrap_or("").trim();
    let ip: IpAddr = entry.parse().map_err(|_| ForwardError::Malformed)?;
    Ok(Some(ip.to_canonical()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header::HeaderValue;

    const PROXY: Forwarding = Forwarding::Proxy {
        peer_when_absent: false,
    };

    fn headers(lines: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for line in lines {
            map.append(FORWARDED_FOR, HeaderValue::from_str(line).unwrap());
        }
        map
    }

    fn loopback() -> SocketAddr {
        "127.0.0.1:40000".parse().unwrap()
    }

    #[test]
    fn the_rightmost_entry_is_the_client_and_its_port_is_unknown() {
        let found = client_addr(&headers(&["203.0.113.9"]), loopback(), PROXY).unwrap();
        assert_eq!(found, "203.0.113.9:0".parse().unwrap());
        let spoofed = headers(&["10.0.0.1, 198.51.100.7", "1.2.3.4,  203.0.113.9 "]);
        let found = client_addr(&spoofed, loopback(), PROXY).unwrap();
        assert_eq!(found.ip(), "203.0.113.9".parse::<IpAddr>().unwrap());
        let v6 = client_addr(&headers(&["2001:db8::7"]), loopback(), PROXY).unwrap();
        assert_eq!(v6.ip(), "2001:db8::7".parse::<IpAddr>().unwrap());
        let mapped = client_addr(&headers(&["::ffff:203.0.113.9"]), loopback(), PROXY).unwrap();
        assert_eq!(mapped.ip(), "203.0.113.9".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn a_missing_or_malformed_header_from_the_proxy_is_refused() {
        assert_eq!(
            client_addr(&HeaderMap::new(), loopback(), PROXY),
            Err(ForwardError::Missing)
        );
        for bad in [
            "",
            "unknown",
            "203.0.113.9:443",
            "[2001:db8::7]",
            "1.2.3.4, ",
        ] {
            assert_eq!(
                client_addr(&headers(&[bad]), loopback(), PROXY),
                Err(ForwardError::Malformed),
                "{bad:?}"
            );
        }
        let dev = Forwarding::Proxy {
            peer_when_absent: true,
        };
        assert_eq!(
            client_addr(&HeaderMap::new(), loopback(), dev),
            Ok(loopback())
        );
        assert_eq!(
            client_addr(&headers(&["junk"]), loopback(), dev),
            Err(ForwardError::Malformed)
        );
    }

    #[test]
    fn only_a_loopback_peer_is_trusted_and_direct_mode_ignores_the_header() {
        let outside: SocketAddr = "198.51.100.2:5555".parse().unwrap();
        let claimed = headers(&["203.0.113.9"]);
        assert_eq!(client_addr(&claimed, outside, PROXY), Ok(outside));
        assert_eq!(
            client_addr(&claimed, loopback(), Forwarding::Direct),
            Ok(loopback())
        );
        let v6_loopback: SocketAddr = "[::1]:9".parse().unwrap();
        let found = client_addr(&claimed, v6_loopback, PROXY).unwrap();
        assert_eq!(found.port(), 0);
    }
}
