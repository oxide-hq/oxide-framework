//! Route-model binding: resolve a model from a route parameter.

use std::collections::HashMap;
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;

use crate::error::OxideHttpError;

/// A model that can be resolved from a route parameter such as `/posts/{post}`.
pub trait RouteBindable<S>: Sized + Send {
    /// The parsed route key (e.g. `u64`, `Uuid`, a slug `String`).
    type Id: FromStr + Send;

    /// The route parameter name, e.g. `"post"` for `/posts/{post}`.
    fn param_name() -> &'static str;

    /// Looks the model up. `Ok(None)` becomes a `404`.
    fn bind_from_state(
        id: Self::Id,
        state: &S,
    ) -> impl Future<Output = Result<Option<Self>, OxideHttpError>> + Send;
}

/// Extractor that resolves `T` from its route parameter, or rejects with `404`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bind<T>(pub T);

impl<T> Deref for Bind<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Bind<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<S, T> FromRequestParts<S> for Bind<T>
where
    S: Send + Sync,
    T: RouteBindable<S>,
{
    type Rejection = OxideHttpError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let name = T::param_name();
        let not_found = |raw: &str| {
            OxideHttpError::NotFound(format!("No query results for model [{name}] {raw}"))
        };

        let Path(params) = Path::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .map_err(|_| not_found(""))?;

        // Fall back to the sole parameter when the route names it differently.
        let sole = params.values().next().filter(|_| params.len() == 1);
        let Some(raw) = params.get(name).or(sole) else {
            return Err(OxideHttpError::Internal(format!(
                "route has no `{{{name}}}` parameter to bind"
            )));
        };

        let id = raw.parse::<T::Id>().map_err(|_| not_found(raw))?;

        T::bind_from_state(id, state)
            .await?
            .map(Bind)
            .ok_or_else(|| not_found(raw))
    }
}

/// A model resolved *within* a parent, so `/users/{user}/posts/{post}` can't reach another
/// user's post (Laravel's scoped bindings).
pub trait ScopedBindable<P, S>: Sized + Send {
    /// The parsed route key of the child.
    type Id: FromStr + Send;

    /// The child's route parameter name, e.g. `"post"`.
    fn param_name() -> &'static str;

    /// Looks the child up among `parent`'s. `Ok(None)` becomes a `404`.
    fn bind_in(
        parent: &P,
        id: Self::Id,
        state: &S,
    ) -> impl Future<Output = Result<Option<Self>, OxideHttpError>> + Send;
}

/// Extractor resolving the parent `P` from its parameter, then `T` scoped to it.
#[derive(Debug, Clone, Copy, Default)]
pub struct BindIn<P, T>(pub P, pub T);

impl<S, P, T> FromRequestParts<S> for BindIn<P, T>
where
    S: Send + Sync,
    P: RouteBindable<S>,
    T: ScopedBindable<P, S>,
{
    type Rejection = OxideHttpError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Bind(parent) = Bind::<P>::from_request_parts(parts, state).await?;
        let name = T::param_name();
        let Path(params) = Path::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .map_err(|_| {
                OxideHttpError::NotFound(format!("No query results for model [{name}]"))
            })?;
        let raw = params.get(name).ok_or_else(|| {
            OxideHttpError::Internal(format!("route has no `{{{name}}}` parameter to bind"))
        })?;
        let not_found =
            || OxideHttpError::NotFound(format!("No query results for model [{name}] {raw}"));
        let id = raw.parse::<T::Id>().map_err(|_| not_found())?;
        let child = T::bind_in(&parent, id, state)
            .await?
            .ok_or_else(not_found)?;
        Ok(Self(parent, child))
    }
}
