//! Service providers: where an application or a package wires itself into the container.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use oxide_config::{ConfigErrors, ConfigSection};
use oxide_http::__private::axum::Router;

use crate::container::{Bindings, Container, ResolveError, ServiceKey};

/// The future a provider's `boot` returns.
pub type BootFuture<'a> = Pin<Box<dyn Future<Output = Result<(), BootError>> + Send + 'a>>;

/// Why the application failed to start.
#[derive(Debug, thiserror::Error)]
pub enum BootError {
    /// Services were declared as required but nothing bound them. Lists all of them.
    #[error("required services are not bound: {}", .0.join(", "))]
    Unbound(Vec<&'static str>),
    /// A provider's `boot` failed.
    #[error("provider `{provider}` failed to boot: {message}")]
    Provider {
        provider: &'static str,
        message: String,
    },
    /// A provider's `boot` returned another kind of error (such as a failed resolve); this names
    /// the provider that raised it.
    #[error("provider `{provider}` failed to boot: {source}")]
    Booting {
        provider: &'static str,
        source: Box<BootError>,
    },
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error(transparent)]
    Config(#[from] ConfigErrors),
}

impl BootError {
    /// A failure in provider `provider`, for use in `boot`.
    pub fn provider(provider: &'static str, error: impl fmt::Display) -> Self {
        Self::Provider {
            provider,
            message: error.to_string(),
        }
    }
}

/// What a provider can see while booting: the finished container and the configuration.
#[derive(Clone, Debug)]
pub struct App {
    container: Container,
    configuration: Option<oxide_config::Configuration>,
}

impl App {
    pub(crate) fn new(
        container: Container,
        configuration: Option<oxide_config::Configuration>,
    ) -> Self {
        Self {
            container,
            configuration,
        }
    }

    pub fn container(&self) -> &Container {
        &self.container
    }

    /// Resolves an application-wide service.
    pub fn resolve<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, ResolveError> {
        self.container.resolve::<T>()
    }

    /// The loaded configuration, if the application was built with one.
    pub fn configuration(&self) -> Option<&oxide_config::Configuration> {
        self.configuration.as_ref()
    }

    /// A configuration section, or an error naming what's missing.
    pub fn config<S: ConfigSection>(&self) -> Result<Arc<S>, BootError> {
        match &self.configuration {
            Some(configuration) => Ok(configuration.require::<S>()?),
            None => Err(BootError::provider(
                "application",
                format!("no configuration was supplied, but `{}` needs it", S::FILE),
            )),
        }
    }
}

/// A unit of application wiring, like a Laravel service provider.
///
/// Startup runs in three steps: every provider's `register` (bind services; nothing resolved
/// yet), then every `boot` in order (the container is complete, so providers may resolve
/// services), then every `routes`.
///
/// A provider that returns keys from `provides` is *deferred*: its `register` runs only when
/// one of those services is first resolved, and its `boot` and `routes` are never called, so
/// use it for self-contained services that need no start-up work.
pub trait ServiceProvider: Send + Sync + 'static {
    /// The name shown in boot errors.
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Binds services. Only bind here: other services may not be registered yet, and a deferred
    /// provider's `register` runs while the container is locked, so it must not resolve anything.
    fn register(&self, _bindings: &mut Bindings) {}

    /// Start-up work that needs other services: warm a cache, check a connection, read config.
    fn boot<'a>(&'a self, _app: &'a App) -> BootFuture<'a> {
        Box::pin(std::future::ready(Ok(())))
    }

    /// Routes the provider contributes, merged into the application's router. Handlers can use
    /// [`Inject`](crate::Inject). Two providers claiming the same path make `merge` panic, as in
    /// Axum.
    fn routes(&self, _app: &App) -> Option<Router> {
        None
    }

    /// The services this provider binds, which makes it deferred when non-empty.
    fn provides(&self) -> Vec<ServiceKey> {
        Vec::new()
    }
}
