use http_body_util::Limited;
use oxide_http::__private::axum::body::Body;
use oxide_http::__private::axum::extract::Request;
use oxide_http::__private::axum::http::{StatusCode, header};

use super::reject;
use crate::middleware::{Middleware, MiddlewareFuture, MwNext};

/// Rejects request bodies larger than a limit with `413 Payload Too Large`.
///
/// A declared `Content-Length` over the limit is refused immediately. Without one (chunked
/// uploads), the body is wrapped so reading past the limit fails, which Axum's extractors report
/// as `413` too.
#[derive(Debug, Clone, Copy)]
pub struct ValidatePostSize {
    max_bytes: usize,
}

impl ValidatePostSize {
    pub fn new(max_bytes: usize) -> Self {
        Self { max_bytes }
    }
}

impl Middleware for ValidatePostSize {
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        _params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(async move {
            let declared = request
                .headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok());
            if declared.is_some_and(|length| length > self.max_bytes) {
                return reject(StatusCode::PAYLOAD_TOO_LARGE, "Payload Too Large");
            }
            let (parts, body) = request.into_parts();
            let request = Request::from_parts(parts, Body::new(Limited::new(body, self.max_bytes)));
            next.run(request).await
        })
    }
}
