//! Route parameter constraints, like Laravel's `->whereNumber('id')`.
//!
//! Each function returns a layer that answers `404 Not Found` when the named path parameter
//! doesn't match, so the handler only ever sees valid values. Use them in a `middleware` block:
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

impl std::fmt::Debug for Constraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Constraint")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::is_uuid;

    #[test]
    fn uuids_need_the_hyphenated_shape() {
        assert!(is_uuid("123e4567-e89b-12d3-a456-426614174000"));
        assert!(!is_uuid("123e4567e89b12d3a456426614174000"));
        assert!(!is_uuid("123e4567-e89b-12d3-a456-42661417400g"));
    }
}
