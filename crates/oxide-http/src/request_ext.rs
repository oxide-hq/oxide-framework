//! Helpers on the incoming request: content negotiation, headers, URL and route questions, like
//! Laravel's `$request->wantsJson()`, `->bearerToken()` and `->routeIs()`.
//!
//! Import [`RequestExt`] (it's in the prelude) and call the methods on `Parts`, which Axum can
//! extract in any handler, or on a whole `Request` in middleware.

use std::sync::Arc;

use axum::extract::{MatchedPath, Request};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HOST, USER_AGENT};
use axum::http::request::Parts;
use axum::http::{Extensions, HeaderMap, Method, Uri};
use percent_encoding::percent_decode_str;

use crate::routing::RouteRegistry;

/// Questions to ask a request. Implemented for [`Parts`] and [`Request`].
pub trait RequestExt {
    fn request_headers(&self) -> &HeaderMap;
    fn request_uri(&self) -> &Uri;
    fn request_method(&self) -> &Method;
    fn request_extensions(&self) -> &Extensions;

    /// A header's value as text.
    fn header(&self, name: &str) -> Option<&str> {
        self.request_headers().get(name)?.to_str().ok()
    }

    fn user_agent(&self) -> Option<&str> {
        self.request_headers().get(USER_AGENT)?.to_str().ok()
    }

    /// The token of an `Authorization: Bearer <token>` header.
    fn bearer_token(&self) -> Option<&str> {
        let value = self.request_headers().get(AUTHORIZATION)?.to_str().ok()?;
        let (scheme, token) = value.split_once(' ')?;
        (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
    }

    /// Whether the request body is JSON (`application/json` or `+json`).
    fn is_json(&self) -> bool {
        self.request_headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(media_type)
            .is_some_and(|media| media == "application/json" || media.ends_with("+json"))
    }

    /// Whether the client's `Accept` header prefers JSON to HTML, as API clients do. Unlike
    /// Laravel's `wantsJson`, `*/*` and a missing header count as JSON: Oxide is API-first, and
    /// only a client that asks for `text/html` ahead of JSON gets pages.
    fn wants_json(&self) -> bool {
        self.prefers(&["application/json", "text/html"]) == Some("application/json")
    }

    /// Whether any of `types` is acceptable according to `Accept`. A missing header accepts
    /// everything.
    fn accepts(&self, types: &[&str]) -> bool {
        let accept = self.header(ACCEPT.as_str());
        types
            .iter()
            .any(|candidate| rank(accept, candidate).is_some())
    }

    /// The best of `types` for this client; see [`negotiate`]. `None` when none is acceptable.
    fn prefers<'a>(&self, types: &[&'a str]) -> Option<&'a str> {
        negotiate(self.header(ACCEPT.as_str()), types)
    }

    /// Whether the path matches any pattern; `*` matches anything, so `admin/*` matches
    /// `/admin/users` and `posts/*/edit` matches `/posts/5/edit`. A leading slash is optional, and
    /// the path is compared percent-decoded, so `café/*` matches `/caf%C3%A9/menu`.
    fn path_is(&self, patterns: &[&str]) -> bool {
        let path = percent_decode_str(self.request_uri().path()).decode_utf8_lossy();
        let path = path.trim_matches('/');
        patterns
            .iter()
            .any(|pattern| wildcard(pattern.trim_matches('/'), path))
    }

    /// The full URL as the client wrote it: the `Host` header plus path and query. The scheme
    /// is `http` unless the request line carried one; behind a proxy use the `Client` extractor
    /// from `oxide-foundation`.
    ///
    /// The `Host` header is chosen by the client. Don't put this URL in an email, a password
    /// reset link or a redirect; build those from a configured application URL.
    fn full_url(&self) -> String {
        let uri = self.request_uri();
        let scheme = uri.scheme_str().unwrap_or("http");
        let host = self
            .header(HOST.as_str())
            .map(str::to_owned)
            .or_else(|| uri.authority().map(ToString::to_string))
            .unwrap_or_default();
        let target = uri
            .path_and_query()
            .map_or(uri.path(), |target| target.as_str());
        format!("{scheme}://{host}{target}")
    }

    /// The name of the matched route (`posts.show`), if it has one and the router was finalized
    /// with `OxideRouter::with_state`.
    fn route_name(&self) -> Option<&str> {
        let registry = self.request_extensions().get::<Arc<RouteRegistry>>()?;
        let matched = self.request_extensions().get::<MatchedPath>()?;
        // Axum answers HEAD with the GET route, which is what the registry holds.
        let method = match self.request_method() {
            &Method::HEAD => &Method::GET,
            other => other,
        };
        registry
            .routes()
            .iter()
            .find(|route| route.path == matched.as_str() && route.method == *method)?
            .name
            .as_deref()
    }

    /// Whether the matched route's name matches a pattern: `posts.*` matches `posts.show`.
    fn route_is(&self, patterns: &[&str]) -> bool {
        self.route_name()
            .is_some_and(|name| patterns.iter().any(|pattern| wildcard(pattern, name)))
    }
}

impl RequestExt for Parts {
    fn request_headers(&self) -> &HeaderMap {
        &self.headers
    }

    fn request_uri(&self) -> &Uri {
        &self.uri
    }

    fn request_method(&self) -> &Method {
        &self.method
    }

    fn request_extensions(&self) -> &Extensions {
        &self.extensions
    }
}

impl RequestExt for Request {
    fn request_headers(&self) -> &HeaderMap {
        self.headers()
    }

    fn request_uri(&self) -> &Uri {
        self.uri()
    }

    fn request_method(&self) -> &Method {
        self.method()
    }

    fn request_extensions(&self) -> &Extensions {
        self.extensions()
    }
}

/// `application/json; charset=utf-8` → `application/json`.
pub(crate) fn media_type(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// Glob matching with `*` standing for any run of characters (including `/`).
fn wildcard(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let [first, rest @ ..] = parts.as_slice() else {
        return false;
    };
    let Some(mut remaining) = text.strip_prefix(first) else {
        return false;
    };
    let Some((last, middle)) = rest.split_last() else {
        return remaining.is_empty();
    };
    for part in middle {
        match remaining.find(part) {
            Some(at) => remaining = &remaining[at + part.len()..],
            None => return false,
        }
    }
    remaining.ends_with(last)
}

/// The best of `types` for an `Accept` header value: the highest `q`, then the one the client
/// listed first, then the order of `types`. A missing header accepts everything, so `types[0]`
/// wins; `None` when nothing is acceptable. A `+json` range (`application/vnd.api+json`) counts
/// as `application/json`.
pub fn negotiate<'a>(accept: Option<&str>, types: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<((u32, usize), &'a str)> = None;
    for candidate in types {
        let Some((quality, position)) = rank(accept, candidate) else {
            continue;
        };
        // Higher quality first, then the earlier range in the header.
        let better = best.is_none_or(|((best_quality, best_position), _)| {
            quality > best_quality || (quality == best_quality && position < best_position)
        });
        if better {
            best = Some(((quality, position), candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// The `(quality in thousandths, position of the range in the header)` the `Accept` header gives
/// `candidate`, from its most specific matching range; `None` when refused (`q=0`) or unmatched.
/// No header means everything is acceptable.
fn rank(accept: Option<&str>, candidate: &str) -> Option<(u32, usize)> {
    let Some(accept) = accept else {
        return Some((1000, 0));
    };
    let candidate = candidate.to_ascii_lowercase();
    let (kind, _) = candidate
        .split_once('/')
        .unwrap_or((candidate.as_str(), ""));
    let mut best: Option<(u8, u32, usize)> = None; // (specificity, quality, position)
    for (position, range) in accept.split(',').enumerate() {
        let mut parts = range.split(';');
        let media = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
        let q = parts
            .find_map(|parameter| {
                parameter
                    .trim()
                    .strip_prefix("q=")?
                    .trim()
                    .parse::<f32>()
                    .ok()
            })
            .map_or(1000, thousandths);
        let specificity = if media == candidate
            || (candidate == "application/json" && media.ends_with("+json"))
        {
            3
        } else if media.strip_suffix("/*") == Some(kind) {
            2
        } else if media == "*/*" || media == "*" {
            1
        } else {
            continue;
        };
        if best.is_none_or(|(best_specificity, ..)| specificity > best_specificity) {
            best = Some((specificity, q, position));
        }
    }
    best.filter(|&(_, q, _)| q > 0)
        .map(|(_, q, position)| (q, position))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped to 0..=1000
fn thousandths(quality: f32) -> u32 {
    (quality.clamp(0.0, 1.0) * 1000.0).round() as u32
}

#[cfg(test)]
mod tests {
    use axum::http::Request as HttpRequest;

    use super::*;

    fn parts(headers: &[(&str, &str)], uri: &str) -> Parts {
        let mut builder = HttpRequest::builder().uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(()).unwrap().into_parts().0
    }

    #[test]
    fn wildcards_match_like_laravel_patterns() {
        assert!(wildcard("admin/*", "admin/users"));
        assert!(wildcard("admin/*", "admin/users/5"));
        assert!(!wildcard("admin/*", "admin"));
        assert!(wildcard("posts/*/edit", "posts/5/edit"));
        assert!(!wildcard("posts/*/edit", "posts/5/show"));
        assert!(wildcard("*", ""));
        assert!(wildcard("", ""));
        assert!(!wildcard("", "x"));
        assert!(wildcard("a*b*c", "a--b--c"));
        assert!(!wildcard("a*b*c", "a--c--b"));
    }

    #[test]
    fn negotiation_follows_q_values_and_specificity() {
        let browser = parts(
            &[("accept", "text/html,application/xhtml+xml,*/*;q=0.8")],
            "/",
        );
        assert!(!browser.wants_json());
        assert_eq!(
            browser.prefers(&["application/json", "text/html"]),
            Some("text/html")
        );

        let api = parts(&[("accept", "application/json")], "/");
        assert!(
            api.wants_json() && api.accepts(&["application/json"]) && !api.accepts(&["text/html"])
        );

        let weighted = parts(
            &[("accept", "text/html;q=0.3, application/json;q=0.9")],
            "/",
        );
        assert!(weighted.wants_json());

        // The most specific range decides: `text/*` is refused outright here.
        let specific = parts(&[("accept", "text/html;q=0, text/*, */*;q=0.1")], "/");
        assert!(!specific.accepts(&["text/html"]));
        assert!(specific.accepts(&["text/plain", "image/png"]));

        // Equal quality: the client's own order decides, then yours.
        let ordered = parts(&[("accept", "text/html, application/json")], "/");
        assert_eq!(
            ordered.prefers(&["application/json", "text/html"]),
            Some("text/html")
        );
        let wildcard_only = parts(&[("accept", "*/*")], "/");
        assert_eq!(
            wildcard_only.prefers(&["application/json", "text/html"]),
            Some("application/json")
        );
        assert!(parts(&[("accept", "application/vnd.api+json")], "/").wants_json());

        // No header accepts everything; the first listed type wins a tie.
        let none = parts(&[], "/");
        assert!(none.accepts(&["anything/else"]));
        assert_eq!(
            none.prefers(&["application/json", "text/html"]),
            Some("application/json")
        );
        assert_eq!(
            parts(&[("accept", "image/png")], "/").prefers(&["application/json"]),
            None
        );
    }

    #[test]
    fn headers_tokens_and_urls() {
        let request = parts(
            &[
                ("authorization", "Bearer abc.def "),
                ("user-agent", "curl/8"),
                ("content-type", "Application/JSON; charset=utf-8"),
                ("host", "app.test"),
            ],
            "/admin/users?page=2",
        );
        assert_eq!(request.bearer_token(), Some("abc.def"));
        assert_eq!(request.user_agent(), Some("curl/8"));
        assert!(request.is_json());
        assert_eq!(request.header("host"), Some("app.test"));
        assert_eq!(request.full_url(), "http://app.test/admin/users?page=2");
        assert!(request.path_is(&["admin/*"]) && request.path_is(&["/admin/users/"]));
        assert!(!request.path_is(&["posts/*"]));

        assert_eq!(
            parts(&[("authorization", "Basic xyz")], "/").bearer_token(),
            None
        );
        assert_eq!(
            parts(&[("authorization", "Bearer ")], "/").bearer_token(),
            None
        );
        assert_eq!(
            parts(&[("authorization", "bearer t")], "/").bearer_token(),
            Some("t")
        );
        let encoded = parts(&[], "/caf%C3%A9/a%20b");
        assert!(encoded.path_is(&["café/*"]) && encoded.path_is(&["café/a b"]));
        assert!(!encoded.path_is(&["cafe/*"]));
        assert!(!parts(&[("content-type", "text/plain")], "/").is_json());
        assert!(parts(&[("content-type", "application/vnd.api+json")], "/").is_json());
        assert_eq!(parts(&[], "http://x.test/a").full_url(), "http://x.test/a");
        assert_eq!(parts(&[], "/").route_name(), None);
    }
}
