//! Reading configuration from handlers and, via facades, from anywhere in a request.

use std::future::ready;
use std::ops::Deref;
use std::sync::Arc;

use oxide_http::__private::axum::extract::{FromRef, FromRequestParts};
use oxide_http::__private::axum::http::request::Parts;
use oxide_http::{OxideHttpError, ServiceContainer};

use crate::loader::{ConfigSection, Configuration};

/// Handler extractor for one configuration section; needs a [`Configuration`] in the state.
///
/// ```ignore
/// async fn about(Config(app): Config<AppConfig>) -> String { app.name.clone() }
/// ```
#[derive(Debug)]
pub struct Config<T>(pub Arc<T>);

impl<T> Deref for Config<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<S, T> FromRequestParts<S> for Config<T>
where
    S: Send + Sync,
    Configuration: FromRef<S>,
    T: ConfigSection,
{
    type Rejection = OxideHttpError;

    fn from_request_parts(
        _parts: &mut Parts,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        ready(
            Configuration::from_ref(state)
                .get::<T>()
                .map(Config)
                .ok_or_else(|| {
                    OxideHttpError::Internal(format!(
                        "configuration section `{}` was not loaded",
                        T::FILE
                    ))
                }),
        )
    }
}

/// Facade lookup of a section from the task-local container, like Laravel's `config('app')`.
///
/// # Panics
/// Panics outside `facade_middleware`, or when the section wasn't registered.
#[track_caller]
pub fn config_of<C, T>() -> Arc<T>
where
    C: ServiceContainer,
    Configuration: FromRef<C>,
    T: ConfigSection,
{
    C::resolve::<Configuration>().get::<T>().unwrap_or_else(|| {
        panic!(
            "configuration section `{}` was not loaded: register it with `ConfigBuilder::section`",
            T::FILE
        )
    })
}
