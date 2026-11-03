//! The dynamic container: typed bindings with singleton, scoped and transient lifetimes.
//!
//! This sits beside `#[derive(ServiceContainer)]`, which stays the zero-cost, compile-time path
//! for services known when you write the state struct. Use this container for what is only known
//! at boot: trait-object contracts, services registered by packages, per-request services.
//!
//! A binding is keyed by type, including `dyn Trait` types. For a contract, declare the trait
//! with `Send + Sync` supertraits so `dyn Trait` is itself `Send + Sync`:
//!
//! ```ignore
//! trait Mailer: Send + Sync { fn send(&self, to: &str); }
//! bindings.singleton_arc::<dyn Mailer>(Arc::new(SmtpMailer::new()));
//! let mailer: Arc<dyn Mailer> = container.resolve()?;
//! ```

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use oxide_http::__private::axum::extract::FromRequestParts;
use oxide_http::__private::axum::http::request::Parts;
use oxide_http::OxideHttpError;

use crate::provider::ServiceProvider;

/// A stored value: an `Arc<T>` behind `Any`, so `T` may be unsized (`dyn Trait`).
type Holder = Arc<dyn Any + Send + Sync>;
type Factory = Arc<dyn Fn(&Scope) -> Result<Holder, ResolveError> + Send + Sync>;

fn holder<T: ?Sized + Send + Sync + 'static>(value: Arc<T>) -> Holder {
    Arc::new(value)
}

fn unholder<T: ?Sized + Send + Sync + 'static>(held: &Holder) -> Option<Arc<T>> {
    held.downcast_ref::<Arc<T>>().cloned()
}

/// Why a service could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// Nothing is bound for the type.
    #[error("no binding for `{0}`")]
    Unbound(&'static str),
    /// A scoped service was requested outside a request scope.
    #[error("`{0}` is scoped to a request and can't be resolved outside one")]
    ScopedOutsideRequest(&'static str),
    /// Services depend on each other in a loop.
    #[error("circular dependency: {0}")]
    Cycle(String),
    /// A factory failed.
    #[error("could not build `{service}`: {message}")]
    Factory {
        service: &'static str,
        message: String,
    },
}

impl ResolveError {
    /// A factory failure for `T`, for use inside `lazy`/`transient`/`scoped` factories.
    pub fn factory<T: ?Sized>(message: impl fmt::Display) -> Self {
        Self::Factory {
            service: std::any::type_name::<T>(),
            message: message.to_string(),
        }
    }
}

/// A binding's identity: the type it resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServiceKey {
    id: TypeId,
    name: &'static str,
}

impl ServiceKey {
    pub(crate) fn type_id(&self) -> TypeId {
        self.id
    }

    pub fn of<T: ?Sized + 'static>() -> Self {
        Self {
            id: TypeId::of::<T>(),
            name: std::any::type_name::<T>(),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

struct SingletonCell {
    /// Read on every resolve, written once: a read lock keeps the hot path concurrent.
    value: RwLock<Option<Holder>>,
    factory: Option<Factory>,
}

#[derive(Clone)]
enum Kind {
    Singleton(Arc<SingletonCell>),
    Transient(Factory),
    Scoped(Factory),
}

/// Collects bindings during `ServiceProvider::register`. A later binding for the same type
/// replaces an earlier one, so applications and tests can override a package's defaults.
#[derive(Default)]
pub struct Bindings {
    entries: HashMap<TypeId, Kind>,
    required: Vec<ServiceKey>,
}

impl fmt::Debug for Bindings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bindings")
            .field("bound", &self.entries.len())
            .field("required", &self.required.len())
            .finish()
    }
}

impl Bindings {
    fn insert<T: ?Sized + 'static>(&mut self, kind: Kind) -> &mut Self {
        self.entries.insert(TypeId::of::<T>(), kind);
        self
    }

    /// Binds one shared instance, resolved as `Arc<T>`.
    pub fn singleton<T: Send + Sync + 'static>(&mut self, value: T) -> &mut Self {
        self.singleton_arc::<T>(Arc::new(value))
    }

    /// Binds one shared instance of `T`, which may be a trait object: pass `Arc<dyn Trait>`.
    pub fn singleton_arc<T: ?Sized + Send + Sync + 'static>(&mut self, value: Arc<T>) -> &mut Self {
        self.insert::<T>(Kind::Singleton(Arc::new(SingletonCell {
            value: RwLock::new(Some(holder(value))),
            factory: None,
        })))
    }

    /// Binds a shared instance built on first use. The factory runs once; services it resolves
    /// from the scope should be singletons too, because the instance outlives any request.
    pub fn lazy<T, F>(&mut self, factory: F) -> &mut Self
    where
        T: ?Sized + Send + Sync + 'static,
        F: Fn(&Scope) -> Result<Arc<T>, ResolveError> + Send + Sync + 'static,
    {
        self.insert::<T>(Kind::Singleton(Arc::new(SingletonCell {
            value: RwLock::new(None),
            factory: Some(wrap(factory)),
        })))
    }

    /// Binds a service built fresh on every resolve.
    pub fn transient<T, F>(&mut self, factory: F) -> &mut Self
    where
        T: ?Sized + Send + Sync + 'static,
        F: Fn(&Scope) -> Result<Arc<T>, ResolveError> + Send + Sync + 'static,
    {
        self.insert::<T>(Kind::Transient(wrap(factory)))
    }

    /// Binds a service built once per request and shared within it.
    pub fn scoped<T, F>(&mut self, factory: F) -> &mut Self
    where
        T: ?Sized + Send + Sync + 'static,
        F: Fn(&Scope) -> Result<Arc<T>, ResolveError> + Send + Sync + 'static,
    {
        self.insert::<T>(Kind::Scoped(wrap(factory)))
    }

    /// Declares that something depends on `T`: `Application::build` fails, listing every missing
    /// binding, if nobody bound it.
    pub fn require<T: ?Sized + 'static>(&mut self) -> &mut Self {
        self.required.push(ServiceKey::of::<T>());
        self
    }

    pub(crate) fn required(&self) -> &[ServiceKey] {
        &self.required
    }

    pub(crate) fn is_bound(&self, id: TypeId) -> bool {
        self.entries.contains_key(&id)
    }
}

fn wrap<T, F>(factory: F) -> Factory
where
    T: ?Sized + Send + Sync + 'static,
    F: Fn(&Scope) -> Result<Arc<T>, ResolveError> + Send + Sync + 'static,
{
    Arc::new(move |scope| factory(scope).map(holder))
}

struct ContainerInner {
    entries: RwLock<HashMap<TypeId, Kind>>,
    /// Providers whose `register` hasn't run yet, by the services they provide.
    deferred: Mutex<HashMap<TypeId, Arc<dyn ServiceProvider>>>,
}

/// The application's typed bindings. Cheap to clone.
#[derive(Clone)]
pub struct Container {
    inner: Arc<ContainerInner>,
}

impl fmt::Debug for Container {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bound = self.inner.entries.read().map_or(0, |entries| entries.len());
        f.debug_struct("Container")
            .field("bound", &bound)
            .finish_non_exhaustive()
    }
}

impl Container {
    pub(crate) fn new(
        bindings: Bindings,
        deferred: HashMap<TypeId, Arc<dyn ServiceProvider>>,
    ) -> Self {
        Self {
            inner: Arc::new(ContainerInner {
                entries: RwLock::new(bindings.entries),
                deferred: Mutex::new(deferred),
            }),
        }
    }

    /// Resolves an application-wide service. Scoped services need a request [`Scope`].
    pub fn resolve<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, ResolveError> {
        Scope::root(self.clone()).resolve::<T>()
    }

    /// Starts a request scope: scoped services are built once inside it.
    pub fn scope(&self) -> Scope {
        Scope::request(self.clone())
    }

    /// Whether a binding exists for `T`, counting providers that haven't registered yet.
    pub fn has<T: ?Sized + 'static>(&self) -> bool {
        let id = TypeId::of::<T>();
        let bound = self
            .inner
            .entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&id);
        bound
            || self
                .inner
                .deferred
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains_key(&id)
    }

    /// The binding for `id`, running a deferred provider's `register` first if it provides it.
    fn lookup(&self, id: TypeId, name: &'static str) -> Result<Kind, ResolveError> {
        let find = || {
            self.inner
                .entries
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&id)
                .cloned()
        };
        if let Some(entry) = find() {
            return Ok(entry);
        }
        // Registration happens under the deferred lock, so a concurrent resolve of the same
        // service waits for it and then finds the binding instead of seeing neither.
        let mut deferred = self
            .inner
            .deferred
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = find() {
            return Ok(entry);
        }
        let Some(provider) = deferred.get(&id).cloned() else {
            return Err(ResolveError::Unbound(name));
        };
        let mut late = Bindings::default();
        provider.register(&mut late);

        let mut entries = self
            .inner
            .entries
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        // A `require` made by the deferred provider fails the resolve that triggered it, and the
        // provider stays deferred so the problem is reported again, not forgotten.
        if let Some(missing) = late.required.iter().find(|key| {
            !late.entries.contains_key(&key.id)
                && !entries.contains_key(&key.id)
                && !deferred.contains_key(&key.id)
        }) {
            return Err(ResolveError::Unbound(missing.name));
        }
        // The provider registers everything it provides at once, and never replaces a binding
        // that already exists: applications override a package's defaults, not the reverse.
        deferred.retain(|_, other| !Arc::ptr_eq(other, &provider));
        for (key, kind) in late.entries {
            entries.entry(key).or_insert(kind);
        }
        entries.get(&id).cloned().ok_or(ResolveError::Unbound(name))
    }
}

thread_local! {
    /// The services being built on this thread, to turn dependency loops into errors.
    static RESOLVING: RefCell<Vec<(TypeId, &'static str)>> = const { RefCell::new(Vec::new()) };
}

struct ResolveGuard;

impl ResolveGuard {
    fn enter(id: TypeId, name: &'static str) -> Result<Self, ResolveError> {
        RESOLVING.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.iter().any(|(seen, _)| *seen == id) {
                let path: Vec<&str> = stack.iter().map(|(_, name)| *name).chain([name]).collect();
                return Err(ResolveError::Cycle(path.join(" -> ")));
            }
            stack.push((id, name));
            Ok(Self)
        })
    }
}

impl Drop for ResolveGuard {
    fn drop(&mut self) {
        RESOLVING.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

struct ScopeInner {
    container: Container,
    request: bool,
    cache: Mutex<HashMap<TypeId, Holder>>,
}

/// Where services are resolved. A request scope caches scoped services; the root scope (from
/// [`Container::resolve`]) refuses them. Cheap to clone.
#[derive(Clone)]
pub struct Scope {
    inner: Arc<ScopeInner>,
}

impl fmt::Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scope")
            .field("request", &self.inner.request)
            .finish_non_exhaustive()
    }
}

impl Scope {
    fn root(container: Container) -> Self {
        Self::with(container, false)
    }

    fn request(container: Container) -> Self {
        Self::with(container, true)
    }

    fn with(container: Container, request: bool) -> Self {
        Self {
            inner: Arc::new(ScopeInner {
                container,
                request,
                cache: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn container(&self) -> &Container {
        &self.inner.container
    }

    /// Resolves `Arc<T>` (or `Arc<dyn Trait>`).
    pub fn resolve<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, ResolveError> {
        let key = ServiceKey::of::<T>();
        let held = self.resolve_holder(key)?;
        unholder::<T>(&held).ok_or(ResolveError::Unbound(key.name))
    }

    fn resolve_holder(&self, key: ServiceKey) -> Result<Holder, ResolveError> {
        let _guard = ResolveGuard::enter(key.id, key.name)?;
        let kind = self.inner.container.lookup(key.id, key.name)?;
        match kind {
            Kind::Singleton(cell) => {
                if let Some(held) = cell
                    .value
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref()
                {
                    return Ok(Arc::clone(held));
                }
                let mut slot = cell.value.write().unwrap_or_else(PoisonError::into_inner);
                if let Some(held) = slot.as_ref() {
                    return Ok(Arc::clone(held));
                }
                let factory = cell
                    .factory
                    .as_ref()
                    .ok_or(ResolveError::Unbound(key.name))?;
                // A singleton outlives every request, so it is built in the root scope: asking
                // for a scoped service there is an error instead of caching one request's data.
                let held = factory(&Scope::root(self.inner.container.clone()))?;
                *slot = Some(Arc::clone(&held));
                Ok(held)
            }
            Kind::Transient(factory) => factory(self),
            Kind::Scoped(factory) => {
                if !self.inner.request {
                    return Err(ResolveError::ScopedOutsideRequest(key.name));
                }
                let cached = self
                    .inner
                    .cache
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&key.id)
                    .cloned();
                if let Some(held) = cached {
                    return Ok(held);
                }
                let built = factory(self)?;
                // If a concurrent resolve built one first, use that, so the request has one.
                let mut cache = self
                    .inner
                    .cache
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                Ok(Arc::clone(cache.entry(key.id).or_insert(built)))
            }
        }
    }
}

/// Handler extractor for a service from the request's scope: `Inject<AppDb>` or
/// `Inject<dyn Mailer>`. Needs the application's scope middleware, which
/// [`Application`](crate::Application) installs for you.
#[derive(Debug)]
pub struct Inject<T: ?Sized>(pub Arc<T>);

impl<T: ?Sized> std::ops::Deref for Inject<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<S, T> FromRequestParts<S> for Inject<T>
where
    S: Send + Sync,
    T: ?Sized + Send + Sync + 'static,
{
    type Rejection = OxideHttpError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        // What went wrong is for the developer (stderr), not for the client in the response body.
        let server_error = |reason: String| {
            eprintln!("oxide-foundation: {reason}");
            OxideHttpError::Internal("Server Error".to_owned())
        };
        let resolved = match parts.extensions.get::<Scope>() {
            Some(scope) => scope
                .resolve::<T>()
                .map(Inject)
                .map_err(|error| server_error(error.to_string())),
            None => Err(server_error(
                "no request scope: build the app with `Application` (or add its scope middleware)"
                    .to_owned(),
            )),
        };
        std::future::ready(resolved)
    }
}
