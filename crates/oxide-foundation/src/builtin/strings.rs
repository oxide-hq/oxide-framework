use http_body_util::LengthLimitError;
use oxide_http::__private::axum::body::{Body, to_bytes};
use oxide_http::__private::axum::extract::Request;
use oxide_http::__private::axum::http::{HeaderValue, StatusCode, header};
use oxide_http::__private::axum::response::Response;
use serde_json::Value;

use super::reject;
use crate::middleware::{Middleware, MiddlewareFuture, MwNext};

/// The largest JSON body these middleware will read into memory.
const DEFAULT_MAX_BODY: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy)]
enum Transform {
    Trim,
    EmptyToNull,
}

/// Rewrites strings anywhere in a JSON value, leaving the subtrees under `except` keys alone.
fn apply(value: &mut Value, transform: Transform, except: &[String]) {
    match value {
        Value::String(text) => match transform {
            Transform::Trim => {
                let trimmed = text.trim();
                if trimmed.len() != text.len() {
                    *text = trimmed.to_owned();
                }
            }
            Transform::EmptyToNull => {
                if text.is_empty() {
                    *value = Value::Null;
                }
            }
        },
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| apply(item, transform, except)),
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if !except.iter().any(|skipped| skipped == key) {
                    apply(child, transform, except);
                }
            }
        }
        _ => {}
    }
}

fn is_json(request: &Request) -> bool {
    request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        })
        .is_some_and(|media| media == "application/json" || media.ends_with("+json"))
}

/// Reads a JSON request body, applies `transform`, and passes the rewritten body on. Bodies that
/// aren't JSON, or don't parse, pass through untouched so the handler's extractor reports them.
async fn rewrite(
    request: Request,
    next: MwNext,
    transform: Transform,
    except: &[String],
    max_body: usize,
) -> Response {
    if !is_json(&request) {
        return next.run(request).await;
    }
    let (mut parts, body) = request.into_parts();
    let bytes = match to_bytes(body, max_body).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let too_large = error
                .into_inner()
                .downcast_ref::<LengthLimitError>()
                .is_some();
            return if too_large {
                reject(StatusCode::PAYLOAD_TOO_LARGE, "Payload Too Large")
            } else {
                reject(
                    StatusCode::BAD_REQUEST,
                    "The request body could not be read.",
                )
            };
        }
    };
    let rewritten = match serde_json::from_slice::<Value>(&bytes) {
        Ok(mut json) => {
            apply(&mut json, transform, except);
            serde_json::to_vec(&json).unwrap_or_else(|_| bytes.to_vec())
        }
        Err(_) => bytes.to_vec(),
    };
    parts
        .headers
        .insert(header::CONTENT_LENGTH, HeaderValue::from(rewritten.len()));
    next.run(Request::from_parts(parts, Body::from(rewritten)))
        .await
}

/// Trims whitespace from every string in a JSON request body, like Laravel's `TrimStrings`.
///
/// Passwords are left alone by default (`password`, `password_confirmation`,
/// `current_password`); [`except`](Self::except) adds more keys. Only `application/json` bodies
/// are rewritten.
#[derive(Debug, Clone)]
pub struct TrimStrings {
    except: Vec<String>,
    max_body: usize,
}

impl Default for TrimStrings {
    fn default() -> Self {
        Self {
            except: ["password", "password_confirmation", "current_password"]
                .map(str::to_owned)
                .to_vec(),
            max_body: DEFAULT_MAX_BODY,
        }
    }
}

impl TrimStrings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Leaves the values under these keys exactly as sent.
    #[must_use]
    pub fn except<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.except.extend(keys.into_iter().map(Into::into));
        self
    }

    /// The most bytes of JSON body to read (default 2 MiB); bigger bodies get `413`.
    #[must_use]
    pub fn max_body(mut self, bytes: usize) -> Self {
        self.max_body = bytes;
        self
    }
}

impl Middleware for TrimStrings {
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        _params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(rewrite(
            request,
            next,
            Transform::Trim,
            &self.except,
            self.max_body,
        ))
    }
}

/// Turns empty strings in a JSON request body into `null`, like Laravel's
/// `ConvertEmptyStringsToNull`, so an optional `Option<String>` field reads `None`.
///
/// Register it after [`TrimStrings`] and a string of only spaces becomes `null` too.
#[derive(Debug, Clone)]
pub struct ConvertEmptyStringsToNull {
    except: Vec<String>,
    max_body: usize,
}

impl Default for ConvertEmptyStringsToNull {
    fn default() -> Self {
        Self {
            except: Vec::new(),
            max_body: DEFAULT_MAX_BODY,
        }
    }
}

impl ConvertEmptyStringsToNull {
    pub fn new() -> Self {
        Self::default()
    }

    /// Leaves the values under these keys exactly as sent.
    #[must_use]
    pub fn except<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.except.extend(keys.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn max_body(mut self, bytes: usize) -> Self {
        self.max_body = bytes;
        self
    }
}

impl Middleware for ConvertEmptyStringsToNull {
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        _params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(rewrite(
            request,
            next,
            Transform::EmptyToNull,
            &self.except,
            self.max_body,
        ))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn transforms_reach_nested_values_but_skip_excepted_keys() {
        let mut value = json!({
            "name": "  Ann  ",
            "password": "  secret  ",
            "tags": [" a ", { "note": " x ", "password": " y " }],
            "age": 3,
            "empty": "",
        });
        apply(&mut value, Transform::Trim, &["password".to_owned()]);
        assert_eq!(
            value,
            json!({
                "name": "Ann",
                "password": "  secret  ",
                "tags": ["a", { "note": "x", "password": " y " }],
                "age": 3,
                "empty": "",
            })
        );
        apply(&mut value, Transform::EmptyToNull, &[]);
        assert_eq!(value["empty"], Value::Null);
        assert_eq!(value["name"], "Ann");
    }
}
