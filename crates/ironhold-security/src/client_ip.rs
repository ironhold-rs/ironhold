//! The client's address, for per-client limits such as the upload cap and
//! the login throttle.

use std::{
    fmt,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    str::FromStr,
    sync::Arc,
};

use axum::{
    extract::{ConnectInfo, FromRequestParts, OptionalFromRequestParts, Request, State},
    http::{HeaderMap, StatusCode, request::Parts},
    middleware::Next,
    response::Response,
};

/// The address of the client that sent the request.
///
/// It's the address of the connection's peer, unless the peer is one of
/// [`SecurityConfig::trusted_proxies`](crate::SecurityConfig::trusted_proxies).
/// Then it's the last address in `X-Forwarded-For` that isn't a trusted
/// proxy, the same rule Rails and Laravel use. Without trusted proxies the
/// header is ignored, because any client can send it.
///
/// Take it as a handler argument. It's missing when the app isn't served by
/// `App::serve` (for example in tests that call the router directly), so
/// use `Option<ClientIp>` where that matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientIp(IpAddr);

impl ClientIp {
    /// Wraps an address, for example in tests.
    pub fn new(ip: IpAddr) -> Self {
        Self(ip.to_canonical())
    }

    /// The client's address.
    pub fn ip(self) -> IpAddr {
        self.0
    }

    /// The address that per-client limits count against: an IPv4 address
    /// as it is, and an IPv6 address reduced to its `/64` network. One
    /// household or server usually gets a whole `/64`, so counting single
    /// IPv6 addresses would let one client look like billions.
    pub fn limit_key(self) -> IpAddr {
        match self.0 {
            IpAddr::V4(v4) => IpAddr::V4(v4),
            IpAddr::V6(v6) => {
                let mut segments = v6.segments();
                segments[4..].fill(0);
                IpAddr::V6(Ipv6Addr::from(segments))
            }
        }
    }

    /// The client address resolved for this request, if any.
    pub fn from_parts(parts: &Parts) -> Option<Self> {
        parts.extensions.get::<Self>().copied()
    }
}

impl fmt::Display for ClientIp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Self::from_parts(parts).ok_or((StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error"))
    }
}

impl<S: Send + Sync> OptionalFromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(Self::from_parts(parts))
    }
}

/// A range of IP addresses in CIDR notation, such as `10.0.0.0/8`, or a
/// single address such as `127.0.0.1`. Used for
/// [`SecurityConfig::trusted_proxies`](crate::SecurityConfig::trusted_proxies).
///
/// ```
/// use ironhold_security::IpRange;
///
/// let range: IpRange = "10.0.0.0/8".parse().unwrap();
/// assert!(range.contains("10.1.2.3".parse().unwrap()));
/// assert!(!range.contains("192.168.0.1".parse().unwrap()));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpRange {
    network: IpAddr,
    prefix: u8,
}

impl IpRange {
    /// Whether `ip` is in this range.
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.network, ip.to_canonical()) {
            (IpAddr::V4(network), IpAddr::V4(ip)) => {
                mask_v4(u32::from(ip), self.prefix) == u32::from(network)
            }
            (IpAddr::V6(network), IpAddr::V6(ip)) => {
                mask_v6(u128::from(ip), self.prefix) == u128::from(network)
            }
            _ => false,
        }
    }
}

impl FromStr for IpRange {
    type Err = InvalidIpRange;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let (addr, prefix) = match s.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (s, None),
        };
        let addr: IpAddr = addr
            .parse::<IpAddr>()
            .map_err(|_| InvalidIpRange)?
            .to_canonical();
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(prefix) => prefix.parse::<u8>().map_err(|_| InvalidIpRange)?,
            None => max,
        };
        if prefix > max {
            return Err(InvalidIpRange);
        }
        let network = match addr {
            IpAddr::V4(v4) => IpAddr::V4(mask_v4(u32::from(v4), prefix).into()),
            IpAddr::V6(v6) => IpAddr::V6(mask_v6(u128::from(v6), prefix).into()),
        };
        Ok(Self { network, prefix })
    }
}

impl fmt::Display for IpRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.prefix)
    }
}

/// The text wasn't an IP address or a CIDR range like `10.0.0.0/8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidIpRange;

impl fmt::Display for InvalidIpRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected an IP address or a CIDR range like `10.0.0.0/8`")
    }
}

impl std::error::Error for InvalidIpRange {}

fn mask_v4(ip: u32, prefix: u8) -> u32 {
    match u32::MAX.checked_shl(32 - u32::from(prefix)) {
        Some(mask) => ip & mask,
        None => 0,
    }
}

fn mask_v6(ip: u128, prefix: u8) -> u128 {
    match u128::MAX.checked_shl(128 - u32::from(prefix)) {
        Some(mask) => ip & mask,
        None => 0,
    }
}

/// Works out the request's [`ClientIp`] from the peer address that
/// `App::serve` records, before any other layer needs it.
pub(crate) async fn resolve_client_ip(
    State(trusted): State<Arc<[IpRange]>>,
    mut request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    if let Some(peer) = peer {
        let client = client_address(peer.to_canonical(), request.headers(), &trusted);
        request.extensions_mut().insert(ClientIp(client));
    }
    next.run(request).await
}

/// The peer, or the address a trusted proxy says it forwarded for. Walks
/// `X-Forwarded-For` from the right (the hop closest to us), skipping
/// trusted proxies, and stops at the first address that isn't one. Entries
/// left of that were written by the client and can't be trusted.
fn client_address(peer: IpAddr, headers: &HeaderMap, trusted: &[IpRange]) -> IpAddr {
    let is_trusted = |ip: IpAddr| trusted.iter().any(|range| range.contains(ip));
    if !is_trusted(peer) {
        return peer;
    }
    let hops: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .collect();
    let mut client = peer;
    for hop in hops.iter().rev() {
        let Some(ip) = parse_hop(hop) else {
            break;
        };
        client = ip;
        if !is_trusted(ip) {
            break;
        }
    }
    client
}

/// An `X-Forwarded-For` entry: `203.0.113.7`, `2001:db8::1`, or either
/// with a port (`203.0.113.7:4711`, `[2001:db8::1]:4711`).
fn parse_hop(hop: &str) -> Option<IpAddr> {
    hop.parse::<IpAddr>()
        .or_else(|_| hop.parse::<SocketAddr>().map(|addr| addr.ip()))
        .ok()
        .map(|ip| ip.to_canonical())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn ranges(list: &[&str]) -> Vec<IpRange> {
        list.iter().map(|s| s.parse().unwrap()).collect()
    }

    fn forwarded(values: &[&'static str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append("x-forwarded-for", HeaderValue::from_static(value));
        }
        headers
    }

    #[test]
    fn ranges_parse_and_match() {
        let v4: IpRange = "10.0.0.0/8".parse().unwrap();
        assert!(v4.contains(ip("10.255.0.1")));
        assert!(!v4.contains(ip("11.0.0.1")));
        assert!(v4.contains(ip("::ffff:10.0.0.1")), "IPv4-mapped IPv6");

        let single: IpRange = "127.0.0.1".parse().unwrap();
        assert!(single.contains(ip("127.0.0.1")));
        assert!(!single.contains(ip("127.0.0.2")));

        let v6: IpRange = "fd00::/8".parse().unwrap();
        assert!(v6.contains(ip("fd12::1")));
        assert!(!v6.contains(ip("fe80::1")));
        assert!(!v6.contains(ip("10.0.0.1")));

        let all: IpRange = "0.0.0.0/0".parse().unwrap();
        assert!(all.contains(ip("203.0.113.9")));

        for bad in ["", "10.0.0.0/33", "::/129", "example.com", "10.0.0.0/x"] {
            assert!(bad.parse::<IpRange>().is_err(), "{bad:?}");
        }
        assert_eq!(
            "10.1.2.3/8".parse::<IpRange>().unwrap().to_string(),
            "10.0.0.0/8"
        );
    }

    #[test]
    fn forwarded_header_is_ignored_unless_the_peer_is_trusted() {
        let headers = forwarded(&["198.51.100.1"]);
        assert_eq!(
            client_address(ip("203.0.113.7"), &headers, &[]),
            ip("203.0.113.7")
        );
        assert_eq!(
            client_address(ip("203.0.113.7"), &headers, &ranges(&["10.0.0.0/8"])),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn trusted_proxies_are_skipped_from_the_right() {
        let trusted = ranges(&["10.0.0.0/8", "127.0.0.1"]);
        // The client wrote a fake first entry; the proxies appended the rest.
        let headers = forwarded(&["1.2.3.4, 198.51.100.1", "10.0.0.2"]);
        assert_eq!(
            client_address(ip("127.0.0.1"), &headers, &trusted),
            ip("198.51.100.1")
        );

        // Ports and brackets are accepted.
        let headers = forwarded(&["[2001:db8::1]:4711"]);
        assert_eq!(
            client_address(ip("10.0.0.1"), &headers, &trusted),
            ip("2001:db8::1")
        );

        // Garbage stops the walk at the last address we trust.
        let headers = forwarded(&["198.51.100.1, nonsense"]);
        assert_eq!(
            client_address(ip("10.0.0.1"), &headers, &trusted),
            ip("10.0.0.1")
        );

        // No header: the proxy itself.
        assert_eq!(
            client_address(ip("10.0.0.1"), &HeaderMap::new(), &trusted),
            ip("10.0.0.1")
        );
    }

    #[test]
    fn ipv6_clients_are_limited_per_64() {
        let a = ClientIp::new(ip("2001:db8:1:2:aaaa::1"));
        let b = ClientIp::new(ip("2001:db8:1:2:bbbb::2"));
        let c = ClientIp::new(ip("2001:db8:1:3::1"));
        assert_eq!(a.limit_key(), b.limit_key());
        assert_ne!(a.limit_key(), c.limit_key());
        let v4 = ClientIp::new(ip("::ffff:203.0.113.7"));
        assert_eq!(v4.limit_key(), ip("203.0.113.7"));
    }
}
