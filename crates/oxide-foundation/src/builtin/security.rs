use oxide_http::__private::axum::extract::Request;
use oxide_http::__private::axum::http::header::{HeaderName, HeaderValue, InvalidHeaderValue};

use crate::middleware::{Middleware, MiddlewareFuture, MwNext};

/// Adds the standard defensive response headers. A header the handler already set is left alone.
///
/// By default: `X-Content-Type-Options: nosniff`, `X-Frame-Options: SAMEORIGIN` and
/// `Referrer-Policy: strict-origin-when-cross-origin`. Add `Strict-Transport-Security`,
/// `Content-Security-Policy` or any other header with the builder methods.
#[derive(Debug, Clone)]
pub struct SecurityHeaders {
    headers: Vec<(HeaderName, HeaderValue)>,
}

impl Default for SecurityHeaders {
    fn default() -> Self {
        Self {
            headers: vec![
                (
                    HeaderName::from_static("x-content-type-options"),
                    HeaderValue::from_static("nosniff"),
                ),
                (
                    HeaderName::from_static("x-frame-options"),
                    HeaderValue::from_static("SAMEORIGIN"),
                ),
                (
                    HeaderName::from_static("referrer-policy"),
                    HeaderValue::from_static("strict-origin-when-cross-origin"),
                ),
            ],
        }
    }
}

impl SecurityHeaders {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) a header.
    #[must_use]
    pub fn header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.retain(|(existing, _)| *existing != name);
        self.headers.push((name, value));
        self
    }

    /// `Strict-Transport-Security`: browsers use HTTPS for `max_age_seconds`. Only send it from
    /// sites that are served over HTTPS.
    #[must_use]
    pub fn hsts(self, max_age_seconds: u64, include_subdomains: bool) -> Self {
        let mut value = format!("max-age={max_age_seconds}");
        if include_subdomains {
            value.push_str("; includeSubDomains");
        }
        match HeaderValue::from_str(&value) {
            Ok(value) => self.header(HeaderName::from_static("strict-transport-security"), value),
            Err(_) => self,
        }
    }

    /// `Content-Security-Policy`.
    pub fn content_security_policy(self, policy: &str) -> Result<Self, InvalidHeaderValue> {
        Ok(self.header(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_str(policy)?,
        ))
    }

    /// `Permissions-Policy`.
    pub fn permissions_policy(self, policy: &str) -> Result<Self, InvalidHeaderValue> {
        Ok(self.header(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_str(policy)?,
        ))
    }
}

impl Middleware for SecurityHeaders {
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        _params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(async move {
            let mut response = next.run(request).await;
            for (name, value) in &self.headers {
                if !response.headers().contains_key(name) {
                    response.headers_mut().insert(name.clone(), value.clone());
                }
            }
            response
        })
    }
}
