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

    /// Whether the client's `Accept` header prefers JSON to HTML, as API clients do.
    fn wants_json(&self) -> bool {
        self.prefers(&["application/json", "text/html"]) == Some("application/json")
    }

    /// Whether any of `types` is acceptable according to `Accept`. A missing header accepts
    /// everything.
    fn accepts(&self, types: &[&str]) -> bool {
        let accept = self.header(ACCEPT.as_str());
        types.iter().any(|candidate| quality(accept, candidate) > 0)
    }

    /// The best of `types` for this client: highest `q`, then the order you listed them in. `None`
    /// when none is acceptable.
    fn prefers<'a>(&self, types: &[&'a str]) -> Option<&'a str> {
        let accept = self.header(ACCEPT.as_str());
        let mut best: Option<(u32, &'a str)> = None;
        for candidate in types {
            let q = quality(accept, candidate);
            if q > 0 && best.is_none_or(|(best_q, _)| q > best_q) {
                best = Some((q, candidate));
            }
        }
        best.map(|(_, candidate)| candidate)
    }

    /// Whether the path matches any pattern; `*` matches anything, so `admin/*` matches
    /// `/admin/users` and `posts/*/edit` matches `/posts/5/edit`. A leading slash is optional.
    fn path_is(&self, patterns: &[&str]) -> bool {
        let path = self.request_uri().path().trim_matches('/');
        patterns
            .iter()
            .any(|pattern| wildcard(pattern.trim_matches('/'), path))
    }

    /// The full URL as the client wrote it: the `Host` header plus path and query. The scheme
    /// is `http` unless the request line carried one; behind a proxy use the `Client` extractor
    /// from `oxide-foundation`.
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
        registry
            .routes()
            .iter()
            .find(|route| route.path == matched.as_str() && route.method == *self.request_method())?
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

/// The quality (thousandths) the `Accept` header gives `candidate`: the most specific matching
/// range wins. No header means everything is acceptable.
fn quality(accept: Option<&str>, candidate: &str) -> u32 {
    let Some(accept) = accept else {
        return 1000;
    };
    let candidate = candidate.to_ascii_lowercase();
    let (kind, _) = candidate
        .split_once('/')
        .unwrap_or((candidate.as_str(), ""));
    let mut best: Option<(u8, u32)> = None; // (specificity, quality)
    for range in accept.split(',') {
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
        let specificity = if media == candidate {
            3
        } else if media.strip_suffix("/*") == Some(kind) {
            2
        } else if media == "*/*" || media == "*" {
            1
        } else {
            continue;
        };
        if best.is_none_or(|(best_specificity, _)| specificity > best_specificity) {
            best = Some((specificity, q));
        }
    }
    best.map_or(0, |(_, q)| q)
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
        assert!(!parts(&[("content-type", "text/plain")], "/").is_json());
        assert!(parts(&[("content-type", "application/vnd.api+json")], "/").is_json());
        assert_eq!(parts(&[], "http://x.test/a").full_url(), "http://x.test/a");
        assert_eq!(parts(&[], "/").route_name(), None);
    }
}
