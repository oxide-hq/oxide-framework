//! A fluent, Laravel-inspired in-memory test client.
//!
//! ```ignore
//! TestClient::new(app)
//!     .put("/posts/1")
//!     .header("X-User-Role", "editor")
//!     .json(&json!({ "title": "Hello world" }))
//!     .send()
//!     .await
//!     .assert_ok()
//!     .assert_json_path("title", "Hello world");
//! ```

use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tower::ServiceExt;

/// Sends requests to a [`Router`] in memory, without binding a socket.
#[derive(Debug, Clone)]
pub struct TestClient {
    router: Router,
}

impl TestClient {
    pub fn new(router: Router) -> Self {
        Self { router }
    }

    pub fn request(&self, method: Method, uri: impl Into<String>) -> TestRequest {
        TestRequest {
            router: self.router.clone(),
            method,
            uri: uri.into(),
            headers: HeaderMap::new(),
            body: Bytes::new(),
        }
    }

    pub fn get(&self, uri: impl Into<String>) -> TestRequest {
        self.request(Method::GET, uri)
    }

    pub fn post(&self, uri: impl Into<String>) -> TestRequest {
        self.request(Method::POST, uri)
    }

    pub fn put(&self, uri: impl Into<String>) -> TestRequest {
        self.request(Method::PUT, uri)
    }

    pub fn patch(&self, uri: impl Into<String>) -> TestRequest {
        self.request(Method::PATCH, uri)
    }

    pub fn delete(&self, uri: impl Into<String>) -> TestRequest {
        self.request(Method::DELETE, uri)
    }
}

/// A request being built by [`TestClient`].
#[derive(Debug)]
#[must_use = "call `.send().await` to dispatch the request"]
pub struct TestRequest {
    router: Router,
    method: Method,
    uri: String,
    headers: HeaderMap,
    body: Bytes,
}

impl TestRequest {
    /// Adds a header.
    ///
    /// # Panics
    /// Panics if the name or value is not a valid HTTP header.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        let name = HeaderName::try_from(name)
            .unwrap_or_else(|e| panic!("invalid header name `{name}`: {e}"));
        let value = HeaderValue::try_from(value)
            .unwrap_or_else(|e| panic!("invalid header value `{value}`: {e}"));
        self.headers.append(name, value);
        self
    }

    /// Sets a JSON body and `Content-Type: application/json`.
    pub fn json<T: Serialize + ?Sized>(mut self, payload: &T) -> Self {
        self.body = serde_json::to_vec(payload)
            .expect("payload must serialize to JSON")
            .into();
        self.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        self
    }

    /// Sets a form-urlencoded body and `Content-Type: application/x-www-form-urlencoded`.
    pub fn form<T: Serialize + ?Sized>(mut self, payload: &T) -> Self {
        self.body = serde_urlencoded::to_string(payload)
            .expect("payload must serialize as form data")
            .into();
        self.headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        self
    }

    /// Sets a raw body (set `Content-Type` yourself with [`TestRequest::header`]).
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    /// Dispatches the request through the router.
    pub async fn send(self) -> TestResponse {
        let mut request = Request::builder()
            .method(self.method)
            .uri(&self.uri)
            .body(Body::from(self.body))
            .unwrap_or_else(|e| panic!("invalid request for `{}`: {e}", self.uri));
        *request.headers_mut() = self.headers;

        let response = self
            .router
            .oneshot(request)
            .await
            .unwrap_or_else(|never| match never {});
        let (parts, body) = response.into_parts();
        let body = body
            .collect()
            .await
            .expect("failed to read response body")
            .to_bytes();

        TestResponse {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }
}

/// A buffered response with fluent assertions. Every assertion returns `&Self` for chaining.
#[derive(Debug, Clone)]
pub struct TestResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl TestResponse {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn bytes(&self) -> &Bytes {
        &self.body
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// Parses the body as JSON.
    ///
    /// # Panics
    /// Panics if the body is not valid JSON.
    #[track_caller]
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("response body is not valid JSON ({e}): {}", self.text()))
    }

    /// Deserializes the body as `T`.
    #[track_caller]
    pub fn json_as<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| {
            panic!(
                "response body could not be deserialized ({e}): {}",
                self.text()
            )
        })
    }

    #[track_caller]
    pub fn assert_status(&self, expected: StatusCode) -> &Self {
        assert_eq!(
            self.status,
            expected,
            "expected status {expected}, got {}. Body: {}",
            self.status,
            self.text()
        );
        self
    }

    #[track_caller]
    pub fn assert_ok(&self) -> &Self {
        self.assert_status(StatusCode::OK)
    }

    #[track_caller]
    pub fn assert_created(&self) -> &Self {
        self.assert_status(StatusCode::CREATED)
    }

    #[track_caller]
    pub fn assert_bad_request(&self) -> &Self {
        self.assert_status(StatusCode::BAD_REQUEST)
    }

    #[track_caller]
    pub fn assert_not_found(&self) -> &Self {
        self.assert_status(StatusCode::NOT_FOUND)
    }

    #[track_caller]
    pub fn assert_forbidden(&self) -> &Self {
        self.assert_status(StatusCode::FORBIDDEN)
    }

    #[track_caller]
    pub fn assert_unprocessable(&self) -> &Self {
        self.assert_status(StatusCode::UNPROCESSABLE_ENTITY)
    }

    /// Asserts the JSON value at `path` equals `expected`.
    ///
    /// `path` is either a JSON pointer (`/data/0/title`) or dot notation (`data.0.title`).
    #[track_caller]
    pub fn assert_json_path(&self, path: &str, expected: impl Into<Value>) -> &Self {
        let expected = expected.into();
        let json = self.json();
        let pointer = to_json_pointer(path);
        match json.pointer(&pointer) {
            Some(actual) => assert_eq!(
                actual, &expected,
                "JSON path `{path}`: expected {expected}, got {actual}. Body: {json}"
            ),
            None => panic!("JSON path `{path}` does not exist. Body: {json}"),
        }
        self
    }

    /// Asserts a `422` response with a non-empty `errors[field]` list.
    #[track_caller]
    pub fn assert_validation_error_for(&self, field: &str) -> &Self {
        self.assert_unprocessable();
        let json = self.json();
        let messages = json
            .get("errors")
            .and_then(|errors| errors.get(field))
            .and_then(Value::as_array);
        assert!(
            messages.is_some_and(|m| !m.is_empty()),
            "expected validation errors for `{field}`. Body: {json}"
        );
        self
    }
}

fn to_json_pointer(path: &str) -> String {
    if path.is_empty() || path.starts_with('/') {
        return path.to_owned();
    }
    let mut pointer = String::with_capacity(path.len() + 1);
    for segment in path.split('.') {
        pointer.push('/');
        pointer.push_str(&segment.replace('~', "~0").replace('/', "~1"));
    }
    pointer
}

#[cfg(test)]
mod tests {
    use super::to_json_pointer;

    #[test]
    fn dot_notation_becomes_pointer() {
        assert_eq!(to_json_pointer("data.0.title"), "/data/0/title");
        assert_eq!(to_json_pointer("/already/pointer"), "/already/pointer");
        assert_eq!(to_json_pointer("a/b"), "/a~1b");
    }
}
