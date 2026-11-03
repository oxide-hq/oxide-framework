//! The [`Application`]: config, providers and routes assembled into one thing you can serve.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use oxide_config::Configuration;
use oxide_http::__private::axum::extract::{Request, State};
use oxide_http::__private::axum::middleware::{self, Next};
use oxide_http::__private::axum::response::Response;
use oxide_http::__private::axum::{self, Router};
use oxide_http::testing::TestClient;

use crate::container::{Bindings, Container, ServiceKey};
use crate::provider::{App, BootError, ServiceProvider};

type Binder = Box<dyn FnOnce(&mut Bindings) + Send>;

/// Collects what the application is made of. Create it with [`Application::builder`].
#[must_use]
pub struct ApplicationBuilder {
    configuration: Option<Configuration>,
    providers: Vec<Arc<dyn ServiceProvider>>,
    binders: Vec<Binder>,
    routes: Vec<Router>,
}

impl fmt::Debug for ApplicationBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApplicationBuilder")
            .field("providers", &self.providers.len())
            .field("routes", &self.routes.len())
            .finish_non_exhaustive()
    }
}

impl ApplicationBuilder {
    /// The loaded configuration. It is bound in the container as `Configuration` too.
    pub fn config(mut self, configuration: Configuration) -> Self {
        self.configuration = Some(configuration);
        self
    }

    /// Adds a provider. Providers register and boot in the order they were added.
    pub fn provider(mut self, provider: impl ServiceProvider) -> Self {
        self.providers.push(Arc::new(provider));
        self
    }

    /// Binds services directly. Runs after every provider's `register`, so a binding made here
    /// overrides a provider's: use it to swap a contract's implementation in tests.
    pub fn bind(mut self, binder: impl FnOnce(&mut Bindings) + Send + 'static) -> Self {
        self.binders.push(Box::new(binder));
        self
    }

    /// Adds routes (for example `router.with_state(ctx).into()` from `routes!`).
    pub fn routes(mut self, router: impl Into<Router>) -> Self {
        self.routes.push(router.into());
        self
    }

    /// Registers every provider, checks the required services, boots the providers in order
    /// and assembles the router.
    pub async fn build(self) -> Result<Application, BootError> {
        let mut bindings = Bindings::default();
        if let Some(configuration) = &self.configuration {
            bindings.singleton(configuration.clone());
        }

        let mut eager = Vec::new();
        let mut deferred = HashMap::new();
        for provider in &self.providers {
            let provides = provider.provides();
            if provides.is_empty() {
                provider.register(&mut bindings);
                eager.push(Arc::clone(provider));
            } else {
                for key in provides {
                    deferred.insert(key.type_id(), Arc::clone(provider));
                }
            }
        }
        for binder in self.binders {
            binder(&mut bindings);
        }

        let missing: Vec<&'static str> = bindings
            .required()
            .iter()
            .filter(|key| {
                !bindings.is_bound(key.type_id()) && !deferred.contains_key(&key.type_id())
            })
            .map(ServiceKey::name)
            .collect();
        if !missing.is_empty() {
            return Err(BootError::Unbound(missing));
        }

        let container = Container::new(bindings, deferred);
        let app = App::new(container.clone(), self.configuration);
        for provider in &eager {
            if let Err(error) = provider.boot(&app).await {
                return Err(match error {
                    BootError::Provider { .. } | BootError::Booting { .. } => error,
                    other => BootError::Booting {
                        provider: provider.name(),
                        source: Box::new(other),
                    },
                });
            }
        }

        let mut router = Router::new();
        for provider in &eager {
            if let Some(routes) = provider.routes(&app) {
                router = router.merge(routes);
            }
        }
        for routes in self.routes {
            router = router.merge(routes);
        }
        let router = router.layer(middleware::from_fn_with_state(container, scope_middleware));
        Ok(Application { app, router })
    }
}

/// Gives every request its own [`Scope`](crate::Scope), so scoped services are built once per
/// request and handlers can use [`Inject`](crate::Inject).
async fn scope_middleware(
    State(container): State<Container>,
    mut request: Request,
    next: Next,
) -> Response {
    request.extensions_mut().insert(container.scope());
    next.run(request).await
}

/// A booted application: the container, the configuration and the assembled router.
#[derive(Debug)]
pub struct Application {
    app: App,
    router: Router,
}

impl Application {
    pub fn builder() -> ApplicationBuilder {
        ApplicationBuilder {
            configuration: None,
            providers: Vec::new(),
            binders: Vec::new(),
            routes: Vec::new(),
        }
    }

    pub fn app(&self) -> &App {
        &self.app
    }

    pub fn container(&self) -> &Container {
        self.app.container()
    }

    /// The router, with the request scope installed.
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    pub fn into_router(self) -> Router {
        self.router
    }

    /// A [`TestClient`] over the application, with providers booted exactly as in production.
    pub fn test_client(&self) -> TestClient {
        TestClient::new(self.router())
    }

    /// Serves on `listener` until the process stops.
    pub async fn serve(self, listener: tokio::net::TcpListener) -> std::io::Result<()> {
        axum::serve(listener, self.router).await
    }
}
