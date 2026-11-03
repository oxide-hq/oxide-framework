//! Request context middleware: a request id on every log line and response.

use oxide_http::__private::axum::extract::Request;
use oxide_http::__private::axum::http::{HeaderName, HeaderValue};
use oxide_http::__private::axum::middleware::Next;
use oxide_http::__private::axum::response::Response;

use crate::context::Context;

const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// Runs the rest of the request inside a [`Context`] holding `request_id`, `method` and `path`,
/// and echoes the id in the `X-Request-Id` response header.
///
/// A client-supplied `X-Request-Id` is kept if it is short and made of letters, digits and
/// `-_.:`; otherwise a fresh UUID is used. `Application` installs this for you.
pub async fn context_middleware(request: Request, next: Next) -> Response {
    let id = request
        .headers()
        .get(&REQUEST_ID)
        .and_then(|value| value.to_str().ok())
        .filter(|id| is_valid_id(id))
        .map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_owned);
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();

    Context::scope(async move {
        Context::add("request_id", id.as_str());
        Context::add("method", method);
        Context::add("path", path);
        let mut response = next.run(request).await;
        if let Ok(value) = HeaderValue::from_str(&id) {
            response.headers_mut().insert(REQUEST_ID, value);
        }
        response
    })
    .await
}

fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

#[cfg(test)]
mod tests {
    use super::is_valid_id;

    #[test]
    fn only_plain_short_ids_are_trusted() {
        assert!(is_valid_id("abc-123_x.y:z"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("has space"));
        assert!(!is_valid_id("inj\nected"));
        assert!(!is_valid_id(&"a".repeat(129)));
    }
}
