//! Route parameter constraints, like Laravel's `->whereNumber('id')`.
//!
//! Each function returns a layer that answers `404 Not Found` when the named path parameter
//! doesn't match, so the handler only ever sees valid values. A route without a parameter of that
//! name is left alone, so a block can cover routes with different parameters; spell the name
//! exactly as in the path, because a typo constrains nothing. Use them in a `middleware` block:
//!
//! ```ignore
//! routes! {
//!     middleware(where_number("post")) {
//!         get("/posts/{post}", show);
//!     }
//!     middleware(where_in("status", &["draft", "published"])) {
//!         get("/posts/by/{status}", by_status);
//!     }
//! }
//! ```

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{RawPathParams, Request};
use axum::middleware::{Next, from_fn};
use axum::response::{IntoResponse, Response};

use tower::util::BoxCloneSyncService;
use tower::{Layer, Service};

use crate::error::OxideHttpError;

type Check = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// A path-parameter check, usable as a layer. Build one with the `where_*` functions.
#[derive(Clone)]
pub struct Constraint {
    name: &'static str,
    check: Check,
}

impl<S> Layer<S> for Constraint
where
    S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + Sync + 'static,
    S::Future: Send + 'static,
{
    type Service = BoxCloneSyncService<Request, Response, Infallible>;

    fn layer(&self, inner: S) -> Self::Service {
        let name = self.name;
        let check = Arc::clone(&self.check);
        let middleware = from_fn(move |params: RawPathParams, request: Request, next: Next| {
            let valid = params
                .iter()
                .filter(|(param, _)| *param == name)
                .all(|(_, value)| check(value));
            async move {
                if valid {
                    next.run(request).await
                } else {
                    OxideHttpError::NotFound("Not Found".to_owned()).into_response()
                }
            }
        });
        BoxCloneSyncService::new(middleware.layer(inner))
    }
}

/// The layer for any predicate: `where_fn("slug", |v| v.len() < 40)`.
pub fn where_fn<F>(name: &'static str, check: F) -> Constraint
where
    F: Fn(&str) -> bool + Send + Sync + 'static,
{
    Constraint {
        name,
        check: Arc::new(check),
    }
}

/// ASCII digits only.
pub fn where_number(name: &'static str) -> Constraint {
    where_fn(name, |v| {
        !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())
    })
}

/// ASCII letters only.
pub fn where_alpha(name: &'static str) -> Constraint {
    where_fn(name, |v| {
        !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphabetic())
    })
}

/// ASCII letters and digits only.
pub fn where_alpha_numeric(name: &'static str) -> Constraint {
    where_fn(name, |v| {
        !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// A hyphenated UUID (8-4-4-4-12 hex digits).
pub fn where_uuid(name: &'static str) -> Constraint {
    where_fn(name, is_uuid)
}

/// One of a fixed set of values, such as an enum's variants.
pub fn where_in(name: &'static str, values: &'static [&'static str]) -> Constraint {
    where_fn(name, move |v| values.contains(&v))
}

fn is_uuid(value: &str) -> bool {
    let groups: Vec<&str> = value.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(group, len)| group.len() == len && group.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Parameters captured from the host by [`domain`]: `{account}.example.com` on
/// `acme.example.com` gives `account = "acme"`. Extract it in a handler.
#[derive(Debug, Clone, Default)]
pub struct DomainParams(HashMap<String, String>);

impl DomainParams {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}

impl<S: Send + Sync> axum::extract::FromRequestParts<S> for DomainParams {
    type Rejection = OxideHttpError;

    fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(Ok(parts
            .extensions
            .get::<Self>()
            .cloned()
            .unwrap_or_default()))
    }
}

/// A layer that only lets requests through whose `Host` matches `pattern` (Laravel's
/// `Route::domain`), answering 404 otherwise. `{name}` captures one label into [`DomainParams`];
/// the port is ignored. Matching is case-insensitive.
///
/// ```ignore
/// routes! {
///     middleware(domain("{account}.example.com")) {
///         get("/dashboard", dashboard);   // dashboard(params: DomainParams)
///     }
/// }
/// ```
pub fn domain(pattern: &'static str) -> DomainLayer {
    DomainLayer(pattern)
}

#[derive(Debug, Clone, Copy)]
pub struct DomainLayer(&'static str);

impl<S> Layer<S> for DomainLayer
where
    S: Service<Request, Response = Response, Error = Infallible> + Clone + Send + Sync + 'static,
    S::Future: Send + 'static,
{
    type Service = BoxCloneSyncService<Request, Response, Infallible>;

    fn layer(&self, inner: S) -> Self::Service {
        let pattern = self.0;
        let middleware = from_fn(move |mut request: Request, next: Next| {
            let host = request
                .headers()
                .get(axum::http::header::HOST)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
                .or_else(|| request.uri().host().map(str::to_owned));
            let captured = host.as_deref().and_then(|host| match_host(pattern, host));
            if let Some(params) = &captured {
                request.extensions_mut().insert(params.clone());
            }
            async move {
                if captured.is_some() {
                    next.run(request).await
                } else {
                    OxideHttpError::NotFound("Not Found".to_owned()).into_response()
                }
            }
        });
        BoxCloneSyncService::new(middleware.layer(inner))
    }
}

/// Matches `host` (port ignored) against label patterns like `{account}.example.com`.
fn match_host(pattern: &str, host: &str) -> Option<DomainParams> {
    let host = host.rsplit_once(':').map_or(host, |(name, port)| {
        if port.bytes().all(|b| b.is_ascii_digit()) {
            name
        } else {
            host
        }
    });
    let host = host.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    let pattern_labels: Vec<&str> = pattern.split('.').collect();
    let host_labels: Vec<&str> = host.split('.').collect();
    if pattern_labels.len() != host_labels.len() {
        return None;
    }
    let mut params = HashMap::new();
    for (expected, actual) in pattern_labels.iter().zip(&host_labels) {
        if let Some(name) = expected.strip_prefix('{').and_then(|n| n.strip_suffix('}')) {
            if actual.is_empty() {
                return None;
            }
            params.insert(name.to_owned(), (*actual).to_owned());
        } else if expected != actual {
            return None;
        }
    }
    Some(DomainParams(params))
}

impl std::fmt::Debug for Constraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Constraint")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::{is_uuid, match_host};

    #[test]
    fn hosts_match_label_patterns() {
        let params = match_host("{account}.example.com", "Acme.example.com:8080").unwrap();
        assert_eq!(params.get("account"), Some("acme"));
        assert!(match_host("{account}.example.com", "example.com").is_none());
        assert!(match_host("{account}.example.com", "a.other.com").is_none());
        assert!(match_host("api.example.com", "api.example.com").is_some());
        assert!(match_host("api.example.com", "www.example.com").is_none());
    }

    #[test]
    fn uuids_need_the_hyphenated_shape() {
        assert!(is_uuid("123e4567-e89b-12d3-a456-426614174000"));
        assert!(!is_uuid("123e4567e89b12d3a456426614174000"));
        assert!(!is_uuid("123e4567-e89b-12d3-a456-42661417400g"));
    }
}
