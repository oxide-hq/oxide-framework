use std::net::{IpAddr, SocketAddr};

use oxide_http::__private::axum::extract::{ConnectInfo, FromRequestParts, Request};
use oxide_http::__private::axum::http::request::Parts;
use oxide_http::__private::axum::http::{HeaderMap, header};
use oxide_http::OxideHttpError;

use crate::middleware::{Middleware, MiddlewareFuture, MwNext};

/// An IP address or a CIDR range (`10.0.0.0/8`, `fd00::/8`).
#[derive(Debug, Clone, Copy)]
struct Network {
    address: IpAddr,
    prefix: u8,
}

impl Network {
    fn parse(text: &str) -> Result<Self, String> {
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address: IpAddr = address
            .trim()
            .parse()
            .map_err(|_| format!("`{text}` is not an IP address or CIDR range"))?;
        let max = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(prefix) => prefix
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|prefix| *prefix <= max)
                .ok_or_else(|| format!("`{text}` has an invalid prefix length (0 to {max})"))?,
            None => max,
        };
        Ok(Self { address, prefix })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (self.address, ip.to_canonical()) {
            (IpAddr::V4(network), IpAddr::V4(ip)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(network) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(network), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(network) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

/// Who really made the request, once trusted proxies are accounted for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// The client's address: the peer itself, or the address a trusted proxy reports.
    pub ip: Option<IpAddr>,
    /// `http` or `https`, from `X-Forwarded-Proto` when a trusted proxy sent it.
    pub scheme: String,
    /// The host the client asked for, from `X-Forwarded-Host` when a trusted proxy sent it.
    pub host: Option<String>,
}

/// Extractor for [`ClientInfo`]: `async fn handler(Client(client): Client) { client.ip }`.
/// Needs [`TrustProxies`] (or `Application`'s `serve`, which provides the peer address).
#[derive(Debug, Clone)]
pub struct Client(pub ClientInfo);

impl<S: Send + Sync> FromRequestParts<S> for Client {
    type Rejection = OxideHttpError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let found = parts.extensions.get::<ClientInfo>().cloned();
        std::future::ready(if let Some(info) = found {
            Ok(Self(info))
        } else {
            oxide_log::tracing::error!("`Client` needs the `TrustProxies` middleware");
            Err(OxideHttpError::Internal("Server Error".to_owned()))
        })
    }
}

/// Works out the real client address, scheme and host behind load balancers and reverse
/// proxies, and stores them as [`ClientInfo`] (read it with the [`Client`] extractor).
///
/// `X-Forwarded-*` headers are believed **only** when the directly connected peer is one of the
/// proxies you list; anyone else could forge them. The client address is the first entry of
/// `X-Forwarded-For`, walking from the right, that isn't itself a trusted proxy. The peer's
/// address comes from `Application::serve`.
#[derive(Debug, Clone)]
pub struct TrustProxies {
    trust_all: bool,
    networks: Vec<Network>,
}

impl TrustProxies {
    /// Trusts these addresses or CIDR ranges: `["127.0.0.1", "10.0.0.0/8", "fd00::/8"]`.
    pub fn new<I, S>(proxies: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let networks = proxies
            .into_iter()
            .map(|proxy| Network::parse(proxy.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            trust_all: false,
            networks,
        })
    }

    /// Trusts every peer. Only for apps that are reachable *only* through a proxy.
    pub fn all() -> Self {
        Self {
            trust_all: true,
            networks: Vec::new(),
        }
    }

    fn trusts(&self, ip: IpAddr) -> bool {
        // `::ffff:10.0.0.5` is the IPv4 address 10.0.0.5 as a dual-stack socket reports it.
        self.trust_all || self.networks.iter().any(|network| network.contains(ip))
    }

    fn client(&self, request: &Request) -> ClientInfo {
        let peer = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip());
        let headers = request.headers();
        let own_host = headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let own_scheme = request.uri().scheme_str().unwrap_or("http").to_owned();
        if !peer.is_some_and(|peer| self.trusts(peer)) {
            return ClientInfo {
                ip: peer,
                scheme: own_scheme,
                host: own_host,
            };
        }

        let mut ip = peer;
        for entry in forwarded(headers, "x-forwarded-for").iter().rev() {
            let Some(forwarded) = parse_forwarded_address(entry) else {
                break;
            };
            ip = Some(forwarded);
            if !self.trusts(forwarded) {
                break;
            }
        }
        let scheme = forwarded(headers, "x-forwarded-proto")
            .first()
            .map(|scheme| scheme.to_ascii_lowercase())
            .filter(|scheme| scheme == "http" || scheme == "https")
            .unwrap_or(own_scheme);
        let host = forwarded(headers, "x-forwarded-host")
            .first()
            .cloned()
            .or(own_host);
        ClientInfo { ip, scheme, host }
    }
}

/// An address from `X-Forwarded-For`, which some proxies write with a port or brackets
/// (`198.51.100.7:51234`, `[2001:db8::1]:443`, `[2001:db8::1]`).
fn parse_forwarded_address(entry: &str) -> Option<IpAddr> {
    let entry = entry.trim();
    if let Ok(ip) = entry.parse::<IpAddr>() {
        return Some(ip.to_canonical());
    }
    if let Ok(socket) = entry.parse::<SocketAddr>() {
        return Some(socket.ip().to_canonical());
    }
    entry
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .and_then(|inner| inner.parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical())
}

/// The comma-separated values of a header (all occurrences, in order).
fn forwarded(headers: &HeaderMap, name: &str) -> Vec<String> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

impl Middleware for TrustProxies {
    fn handle<'a>(
        &'a self,
        mut request: Request,
        next: MwNext,
        _params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(async move {
            let client = self.client(&request);
            request.extensions_mut().insert(client);
            next.run(request).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networks_match_addresses_and_ranges() {
        let net = |text| Network::parse(text).unwrap();
        assert!(net("10.0.0.0/8").contains("10.255.1.2".parse().unwrap()));
        assert!(!net("10.0.0.0/8").contains("11.0.0.1".parse().unwrap()));
        assert!(net("127.0.0.1").contains("127.0.0.1".parse().unwrap()));
        assert!(!net("127.0.0.1").contains("127.0.0.2".parse().unwrap()));
        assert!(net("0.0.0.0/0").contains("8.8.8.8".parse().unwrap()));
        assert!(net("fd00::/8").contains("fdab::1".parse().unwrap()));
        assert!(!net("fd00::/8").contains("fe80::1".parse().unwrap()));
        assert!(
            !net("10.0.0.0/8").contains("::1".parse().unwrap()),
            "families don't mix"
        );
        for bad in ["nope", "10.0.0.0/33", "::/129", "10.0.0.0/x"] {
            assert!(Network::parse(bad).is_err(), "{bad}");
        }
    }
}
