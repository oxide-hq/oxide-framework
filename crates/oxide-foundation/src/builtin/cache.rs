use oxide_http::__private::axum::extract::Request;
use oxide_http::__private::axum::http::{HeaderValue, Method, header};

use crate::middleware::{Middleware, MiddlewareFuture, MwNext};

/// Sets `Cache-Control` on successful `GET` and `HEAD` responses that don't set it themselves.
///
/// ```ignore
/// SetCacheHeaders::new().public().max_age(3600)   // Cache-Control: public, max-age=3600
/// SetCacheHeaders::new().no_store()               // Cache-Control: no-store
/// ```
#[derive(Debug, Clone, Default)]
pub struct SetCacheHeaders {
    directives: Vec<String>,
}

impl SetCacheHeaders {
    pub fn new() -> Self {
        Self::default()
    }

    fn with(mut self, directive: String) -> Self {
        self.directives.push(directive);
        self
    }

    /// Any cache, including shared ones, may store the response.
    #[must_use]
    pub fn public(self) -> Self {
        self.with("public".into())
    }

    /// Only the user's own cache may store the response.
    #[must_use]
    pub fn private(self) -> Self {
        self.with("private".into())
    }

    #[must_use]
    pub fn max_age(self, seconds: u64) -> Self {
        self.with(format!("max-age={seconds}"))
    }

    /// Like `max_age`, for shared caches.
    #[must_use]
    pub fn s_maxage(self, seconds: u64) -> Self {
        self.with(format!("s-maxage={seconds}"))
    }

    #[must_use]
    pub fn no_cache(self) -> Self {
        self.with("no-cache".into())
    }

    #[must_use]
    pub fn no_store(self) -> Self {
        self.with("no-store".into())
    }

    #[must_use]
    pub fn must_revalidate(self) -> Self {
        self.with("must-revalidate".into())
    }

    #[must_use]
    pub fn immutable(self) -> Self {
        self.with("immutable".into())
    }

    fn value(&self) -> Option<HeaderValue> {
        if self.directives.is_empty() {
            return None;
        }
        HeaderValue::from_str(&self.directives.join(", ")).ok()
    }
}

impl Middleware for SetCacheHeaders {
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        _params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(async move {
            let cacheable = matches!(*request.method(), Method::GET | Method::HEAD);
            let mut response = next.run(request).await;
            if cacheable
                && response.status().is_success()
                && !response.headers().contains_key(header::CACHE_CONTROL)
                && let Some(value) = self.value()
            {
                response.headers_mut().insert(header::CACHE_CONTROL, value);
            }
            response
        })
    }
}
