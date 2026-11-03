use oxide_http::__private::axum::extract::Request;
use oxide_http::__private::axum::http::{StatusCode, header};

use super::reject;
use crate::middleware::{Middleware, MiddlewareFuture, MwNext};

/// Answers `400 Bad Request` unless the request's `Host` is one you expect, which defeats
/// host-header attacks (poisoned password-reset links, cache poisoning).
///
/// Patterns are exact hosts (`example.com`), subdomain wildcards (`*.example.com`, which does
/// not match `example.com` itself) or `*` for any. Ports are ignored; matching is
/// case-insensitive.
#[derive(Debug, Clone)]
pub struct TrustHosts {
    patterns: Vec<String>,
}

impl TrustHosts {
    pub fn new<I, S>(patterns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            patterns: patterns
                .into_iter()
                .map(|pattern| pattern.as_ref().to_ascii_lowercase())
                .collect(),
        }
    }

    fn trusts(&self, host: &str) -> bool {
        self.patterns
            .iter()
            .any(|pattern| match pattern.strip_prefix("*.") {
                Some(suffix) => host
                    .strip_suffix(suffix)
                    .is_some_and(|prefix| prefix.ends_with('.') && prefix.len() > 1),
                None => pattern == "*" || pattern == host,
            })
    }
}

/// The host without its port: `Example.com:8080` → `example.com`, `[::1]:3000` → `[::1]`.
fn host_name(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    if value.starts_with('[') {
        return value
            .find(']')
            .map_or(value.clone(), |end| value[..=end].to_owned());
    }
    value
        .rsplit_once(':')
        .map_or(value.clone(), |(host, _port)| host.to_owned())
}

impl Middleware for TrustHosts {
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        _params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(async move {
            let host = request
                .headers()
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
                .or_else(|| request.uri().authority().map(ToString::to_string));
            match host {
                Some(host) if self.trusts(&host_name(&host)) => next.run(request).await,
                _ => reject(StatusCode::BAD_REQUEST, "Invalid host."),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_hosts_not_lookalikes() {
        let hosts = TrustHosts::new(["Example.com", "*.example.org"]);
        for (host, trusted) in [
            ("example.com", true),
            ("example.com:8080", true),
            ("EXAMPLE.COM", true),
            ("api.example.org", true),
            ("a.b.example.org", true),
            ("example.org", false),
            ("evilexample.org", false),
            ("example.com.evil.net", false),
            (".example.org", false),
            ("", false),
        ] {
            assert_eq!(hosts.trusts(&host_name(host)), trusted, "{host}");
        }
        assert!(TrustHosts::new(["*"]).trusts("anything"));
        assert_eq!(host_name("[::1]:3000"), "[::1]");
        assert_eq!(host_name("localhost"), "localhost");
    }
}
