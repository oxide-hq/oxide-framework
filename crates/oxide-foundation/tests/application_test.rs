//! Application bootstrap: providers, binding lifetimes, contracts, deferral and configuration.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use oxide_config::{Config, Configuration};
use oxide_foundation::prelude::*;
use oxide_http::__private::axum::Router;
use oxide_http::__private::axum::routing::get;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Log = Arc<Mutex<Vec<String>>>;

fn log(entries: &Log) -> Vec<String> {
    entries.lock().unwrap().clone()
}

// ---- a contract with two implementations
trait Mailer: Send + Sync {
    fn send(&self, to: &str) -> String;
}

struct SmtpMailer;

impl Mailer for SmtpMailer {
    fn send(&self, to: &str) -> String {
        format!("smtp:{to}")
    }
}

struct LogMailer;

impl Mailer for LogMailer {
    fn send(&self, to: &str) -> String {
        format!("log:{to}")
    }
}

async fn send_mail(mailer: Inject<dyn Mailer>) -> String {
    mailer.send("ann")
}

struct MailProvider;

impl ServiceProvider for MailProvider {
    fn register(&self, bindings: &mut Bindings) {
        bindings.singleton_arc::<dyn Mailer>(Arc::new(SmtpMailer));
    }

    fn routes(&self, _app: &App) -> Option<Router> {
        Some(Router::new().route("/mail", get(send_mail)))
    }
}

#[tokio::test]
async fn a_contract_is_injected_and_can_be_swapped() {
    let app = Application::builder()
        .provider(MailProvider)
        .build()
        .await
        .unwrap();
    let response = app.test_client().get("/mail").send().await;
    response.assert_ok();
    assert_eq!(response.text(), "smtp:ann");

    // A binding made on the builder runs after every `register`, so it wins.
    let app = Application::builder()
        .provider(MailProvider)
        .bind(|bindings| {
            bindings.singleton_arc::<dyn Mailer>(Arc::new(LogMailer));
        })
        .build()
        .await
        .unwrap();
    assert_eq!(
        app.test_client().get("/mail").send().await.text(),
        "log:ann"
    );
    assert_eq!(
        app.container().resolve::<dyn Mailer>().unwrap().send("x"),
        "log:x"
    );
}

// ---- lifetimes
static NEXT_ID: AtomicU32 = AtomicU32::new(0);

#[derive(Debug)]
struct RequestId(u32);

async fn twice(first: Inject<RequestId>, second: Inject<RequestId>) -> String {
    format!("{}-{}", (*first).0, (*second).0)
}

#[tokio::test]
async fn scoped_services_are_built_once_per_request() {
    let app = Application::builder()
        .bind(|bindings| {
            bindings.scoped(|_| Ok(Arc::new(RequestId(NEXT_ID.fetch_add(1, Ordering::SeqCst)))));
        })
        .routes(Router::new().route("/id", get(twice)))
        .build()
        .await
        .unwrap();
    let client = app.test_client();
    let first = client.get("/id").send().await.text();
    let second = client.get("/id").send().await.text();
    let (a, b) = first.split_once('-').unwrap();
    assert_eq!(a, b, "one instance inside a request: {first}");
    assert_ne!(first, second, "a new instance for the next request");
}

#[tokio::test]
async fn singleton_transient_and_lazy_lifetimes() {
    let builds = Arc::new(AtomicU32::new(0));
    let counted = Arc::clone(&builds);
    let app = Application::builder()
        .bind(move |bindings| {
            bindings.singleton(String::from("shared"));
            bindings.transient(|_| Ok(Arc::new(NEXT_ID.fetch_add(1, Ordering::SeqCst))));
            bindings.lazy(move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(vec![1_u8, 2, 3]))
            });
        })
        .build()
        .await
        .unwrap();
    let container = app.container();

    let a = container.resolve::<String>().unwrap();
    assert!(Arc::ptr_eq(&a, &container.resolve::<String>().unwrap()));
    assert_ne!(
        container.resolve::<u32>().unwrap(),
        container.resolve::<u32>().unwrap()
    );

    assert_eq!(
        builds.load(Ordering::SeqCst),
        0,
        "lazy singletons wait for first use"
    );
    let first = container.resolve::<Vec<u8>>().unwrap();
    let second = container.resolve::<Vec<u8>>().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(builds.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn resolve_errors_are_specific() {
    let app = Application::builder()
        .bind(|bindings| {
            bindings.scoped(|_| Ok(Arc::new(RequestId(0))));
            bindings.lazy::<u8, _>(|_| Err(ResolveError::factory::<u8>("no luck")));
            bindings.lazy::<u16, _>(|scope| {
                scope
                    .resolve::<u32>()
                    .map(|n| Arc::new(u16::try_from(*n).unwrap_or(0)))
            });
            bindings
                .lazy::<u32, _>(|scope| scope.resolve::<u16>().map(|n| Arc::new(u32::from(*n))));
        })
        .build()
        .await
        .unwrap();
    let container = app.container();

    assert!(matches!(
        container.resolve::<RequestId>(),
        Err(ResolveError::ScopedOutsideRequest(_))
    ));
    assert!(container.scope().resolve::<RequestId>().is_ok());
    assert!(matches!(
        container.resolve::<i64>(),
        Err(ResolveError::Unbound(_))
    ));
    let failed = container.resolve::<u8>().unwrap_err();
    assert!(failed.to_string().contains("no luck"), "{failed}");

    let Err(ResolveError::Cycle(path)) = container.resolve::<u16>() else {
        panic!("expected a cycle");
    };
    assert!(path.contains("u16") && path.contains("u32"), "{path}");
    // The guard is released: the same lookup reports the same cycle again, not a stale state.
    assert!(matches!(
        container.resolve::<u16>(),
        Err(ResolveError::Cycle(_))
    ));
}

#[tokio::test]
async fn an_unbound_service_is_an_opaque_500() {
    async fn handler(_missing: Inject<i64>) -> &'static str {
        "unreachable"
    }
    let app = Application::builder()
        .routes(Router::new().route("/missing", get(handler)))
        .build()
        .await
        .unwrap();
    let response = app.test_client().get("/missing").send().await;
    assert_eq!(response.status().as_u16(), 500);
    assert!(!response.text().contains("i64"), "{}", response.text());

    // Without the application's scope middleware there is no scope at all.
    let bare = Router::new().route("/missing", get(handler));
    let response = oxide_http::testing::TestClient::new(bare)
        .get("/missing")
        .send()
        .await;
    assert_eq!(response.status().as_u16(), 500);
    assert!(!response.text().contains("scope"), "{}", response.text());
}

// ---- provider lifecycle
struct Recorder {
    name: &'static str,
    log: Log,
}

impl ServiceProvider for Recorder {
    fn name(&self) -> &'static str {
        self.name
    }

    fn register(&self, bindings: &mut Bindings) {
        self.log
            .lock()
            .unwrap()
            .push(format!("register {}", self.name));
        let name = self.name;
        bindings.lazy(move |_| Ok(Arc::new(format!("service of {name}"))));
    }

    fn boot<'a>(&'a self, app: &'a App) -> BootFuture<'a> {
        Box::pin(async move {
            // Every provider has registered by now, so `app` sees all services.
            let service = app.resolve::<String>()?;
            self.log
                .lock()
                .unwrap()
                .push(format!("boot {} sees {service}", self.name));
            Ok(())
        })
    }

    fn routes(&self, _app: &App) -> Option<Router> {
        let name = self.name;
        Some(Router::new().route(&format!("/{name}"), get(move || async move { name })))
    }
}

#[tokio::test]
async fn providers_register_then_boot_in_order_and_contribute_routes() {
    let entries: Log = Arc::default();
    let app = Application::builder()
        .provider(Recorder {
            name: "a",
            log: Arc::clone(&entries),
        })
        .provider(Recorder {
            name: "b",
            log: Arc::clone(&entries),
        })
        .build()
        .await
        .unwrap();
    assert_eq!(
        log(&entries),
        [
            "register a",
            "register b",
            // The later registration replaced the earlier binding for the same type.
            "boot a sees service of b",
            "boot b sees service of b",
        ]
    );
    let client = app.test_client();
    assert_eq!(client.get("/a").send().await.text(), "a");
    assert_eq!(client.get("/b").send().await.text(), "b");
}

struct Failing;

impl ServiceProvider for Failing {
    fn name(&self) -> &'static str {
        "failing"
    }

    fn boot<'a>(&'a self, _app: &'a App) -> BootFuture<'a> {
        Box::pin(std::future::ready(Err(BootError::provider(
            "failing",
            "database is down",
        ))))
    }
}

#[tokio::test]
async fn a_failing_boot_stops_the_build_and_names_the_provider() {
    let error = Application::builder()
        .provider(Failing)
        .build()
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("provider `failing` failed to boot: database is down"),
        "{error}"
    );
}

struct NeedsMailer;

impl ServiceProvider for NeedsMailer {
    fn register(&self, bindings: &mut Bindings) {
        bindings.require::<dyn Mailer>();
        bindings.require::<RequestId>();
    }
}

#[tokio::test]
async fn missing_required_services_are_all_listed_at_build() {
    let error = Application::builder()
        .provider(NeedsMailer)
        .build()
        .await
        .unwrap_err();
    let BootError::Unbound(missing) = &error else {
        panic!("expected Unbound, got {error}");
    };
    assert_eq!(missing.len(), 2, "{error}");
    assert!(error.to_string().contains("Mailer") && error.to_string().contains("RequestId"));

    // Binding one of them leaves only the other.
    let error = Application::builder()
        .provider(NeedsMailer)
        .bind(|bindings| {
            bindings.singleton_arc::<dyn Mailer>(Arc::new(LogMailer));
        })
        .build()
        .await
        .unwrap_err();
    assert!(
        matches!(&error, BootError::Unbound(missing) if missing.len() == 1),
        "{error}"
    );

    // Both bound: fine.
    Application::builder()
        .provider(NeedsMailer)
        .bind(|bindings| {
            bindings.singleton_arc::<dyn Mailer>(Arc::new(LogMailer));
            bindings.singleton(RequestId(1));
        })
        .build()
        .await
        .unwrap();
}

struct Deferred {
    log: Log,
}

impl ServiceProvider for Deferred {
    fn register(&self, bindings: &mut Bindings) {
        self.log.lock().unwrap().push("register deferred".into());
        bindings.singleton(7_u64);
    }

    fn boot<'a>(&'a self, _app: &'a App) -> BootFuture<'a> {
        self.log.lock().unwrap().push("boot deferred".into());
        Box::pin(std::future::ready(Ok(())))
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![ServiceKey::of::<u64>()]
    }
}

#[tokio::test]
async fn deferred_providers_register_on_first_use_and_never_boot() {
    let entries: Log = Arc::default();
    let app = Application::builder()
        .provider(Deferred {
            log: Arc::clone(&entries),
        })
        .build()
        .await
        .unwrap();
    assert!(
        log(&entries).is_empty(),
        "nothing runs at build: {:?}",
        log(&entries)
    );
    assert!(app.container().has::<u64>());
    assert!(!app.container().has::<u32>());

    assert_eq!(*app.container().resolve::<u64>().unwrap(), 7);
    assert_eq!(*app.container().resolve::<u64>().unwrap(), 7);
    assert_eq!(log(&entries), ["register deferred"]);
}

// ---- configuration
#[allow(dead_code)]
#[derive(Clone, Deserialize, Config)]
#[config(file = "app")]
struct AppConfig {
    #[config(default = "Oxide")]
    name: String,
}

struct ReadsConfig {
    seen: Log,
}

impl ServiceProvider for ReadsConfig {
    fn boot<'a>(&'a self, app: &'a App) -> BootFuture<'a> {
        Box::pin(async move {
            let config = app.config::<AppConfig>()?;
            self.seen.lock().unwrap().push(config.name.clone());
            Ok(())
        })
    }
}

#[tokio::test]
async fn providers_read_configuration_and_missing_pieces_are_errors() {
    let configuration = Configuration::builder()
        .vars(Vec::<(String, String)>::new())
        .section::<AppConfig>()
        .load()
        .unwrap();
    let seen: Log = Arc::default();
    let app = Application::builder()
        .config(configuration.clone())
        .provider(ReadsConfig {
            seen: Arc::clone(&seen),
        })
        .build()
        .await
        .unwrap();
    assert_eq!(log(&seen), ["Oxide"]);
    // The configuration is also a bound service.
    assert!(app.container().resolve::<Configuration>().is_ok());
    assert!(app.app().configuration().is_some());

    // A section that was never registered is a configuration error, not a panic.
    let bare = Configuration::builder()
        .vars(Vec::<(String, String)>::new())
        .load()
        .unwrap();
    let error = Application::builder()
        .config(bare)
        .provider(ReadsConfig {
            seen: Arc::default(),
        })
        .build()
        .await
        .unwrap_err();
    assert!(
        matches!(&error, BootError::Booting { source, .. } if matches!(**source, BootError::Config(_))),
        "{error}"
    );
    assert!(error.to_string().contains("ReadsConfig"), "{error}");

    // No configuration at all is reported too.
    let error = Application::builder()
        .provider(ReadsConfig {
            seen: Arc::default(),
        })
        .build()
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no configuration"), "{error}");
}

// ---- serving
#[tokio::test]
async fn serve_answers_real_connections() {
    let app = Application::builder()
        .provider(MailProvider)
        .build()
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(app.serve(listener));

    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(b"GET /mail HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.ends_with("smtp:ann"), "{reply}");
    server.abort();
}

#[test]
fn debug_output_is_useful_and_cheap() {
    let builder = Application::builder().routes(Router::new());
    assert!(format!("{builder:?}").contains("ApplicationBuilder"));
    let mut bindings = Bindings::default();
    bindings.singleton(1_u8);
    assert!(format!("{bindings:?}").contains("bound: 1"));
    assert!(format!("{:?}", ServiceKey::of::<u8>()).contains("u8"));
}

// ---- review regressions
#[tokio::test]
async fn singletons_are_built_outside_any_request_scope() {
    let app = Application::builder()
        .bind(|bindings| {
            bindings.scoped(|_| Ok(Arc::new(RequestId(5))));
            // A singleton that depends on a request-scoped service would cache one request's data.
            bindings.lazy::<String, _>(|scope| {
                scope
                    .resolve::<RequestId>()
                    .map(|id| Arc::new(format!("user {}", id.0)))
            });
        })
        .build()
        .await
        .unwrap();
    let scope = app.container().scope();
    assert!(matches!(
        scope.resolve::<String>(),
        Err(ResolveError::ScopedOutsideRequest(_))
    ));
}

#[tokio::test]
async fn a_failing_resolve_in_boot_names_the_provider() {
    struct NeedsMissing;

    impl ServiceProvider for NeedsMissing {
        fn name(&self) -> &'static str {
            "needs-missing"
        }

        fn boot<'a>(&'a self, app: &'a App) -> BootFuture<'a> {
            Box::pin(async move {
                app.resolve::<i64>()?;
                Ok(())
            })
        }
    }
    let error = Application::builder()
        .provider(NeedsMissing)
        .build()
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("provider `needs-missing` failed to boot"),
        "{error}"
    );
    assert!(error.to_string().contains("no binding"), "{error}");
}

struct SlowDeferred;

impl ServiceProvider for SlowDeferred {
    fn register(&self, bindings: &mut Bindings) {
        std::thread::sleep(std::time::Duration::from_millis(40));
        bindings.singleton(7_u64);
        // Bound by the provider but not listed in `provides`.
        bindings.singleton(1_u8);
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![ServiceKey::of::<u64>()]
    }
}

#[tokio::test]
async fn concurrent_first_resolves_of_a_deferred_service_all_succeed() {
    let app = Application::builder()
        .provider(SlowDeferred)
        .build()
        .await
        .unwrap();
    let container = app.container().clone();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let container = container.clone();
            std::thread::spawn(move || container.resolve::<u64>().map(|n| *n))
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap().unwrap(), 7);
    }
}

#[tokio::test]
async fn a_deferred_provider_never_replaces_an_application_override() {
    let app = Application::builder()
        .provider(SlowDeferred)
        .bind(|bindings| {
            bindings.singleton(9_u8);
        })
        .build()
        .await
        .unwrap();
    assert_eq!(*app.container().resolve::<u64>().unwrap(), 7);
    assert_eq!(*app.container().resolve::<u8>().unwrap(), 9);
}

#[tokio::test]
async fn a_deferred_providers_unmet_requirement_fails_its_first_resolve() {
    struct DeferredNeedsMailer;

    impl ServiceProvider for DeferredNeedsMailer {
        fn register(&self, bindings: &mut Bindings) {
            bindings.require::<dyn Mailer>();
            bindings.singleton(3_u32);
        }

        fn provides(&self) -> Vec<ServiceKey> {
            vec![ServiceKey::of::<u32>()]
        }
    }
    let app = Application::builder()
        .provider(DeferredNeedsMailer)
        .build()
        .await
        .unwrap();
    for _ in 0..2 {
        let error = app.container().resolve::<u32>().unwrap_err();
        assert!(error.to_string().contains("Mailer"), "{error}");
    }
    // Once the requirement is met the provider registers normally.
    let app = Application::builder()
        .provider(DeferredNeedsMailer)
        .bind(|bindings| {
            bindings.singleton_arc::<dyn Mailer>(Arc::new(LogMailer));
        })
        .build()
        .await
        .unwrap();
    assert_eq!(*app.container().resolve::<u32>().unwrap(), 3);
}

#[tokio::test]
async fn concurrent_resolves_share_one_scoped_instance_and_one_singleton() {
    let singleton_builds = Arc::new(AtomicU32::new(0));
    let counted = Arc::clone(&singleton_builds);
    let app = Application::builder()
        .bind(move |bindings| {
            bindings.scoped(|_| {
                std::thread::sleep(std::time::Duration::from_millis(30));
                Ok(Arc::new(RequestId(NEXT_ID.fetch_add(1, Ordering::SeqCst))))
            });
            bindings.lazy(move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(30));
                Ok(Arc::new(String::from("pool")))
            });
        })
        .build()
        .await
        .unwrap();
    let scope = app.container().scope();
    let threads: Vec<_> = (0..6)
        .map(|_| {
            let scope = scope.clone();
            std::thread::spawn(move || {
                (
                    scope.resolve::<RequestId>().unwrap(),
                    scope.resolve::<String>().unwrap(),
                )
            })
        })
        .collect();
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(results.iter().all(|(id, _)| Arc::ptr_eq(id, &results[0].0)));
    assert!(
        results
            .iter()
            .all(|(_, pool)| Arc::ptr_eq(pool, &results[0].1))
    );
    assert_eq!(singleton_builds.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn every_response_carries_a_request_id() {
    let app = Application::builder()
        .provider(MailProvider)
        .build()
        .await
        .unwrap();
    let response = app.test_client().get("/mail").send().await;
    response.assert_ok();
    let id = response
        .headers()
        .get("x-request-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert_eq!(id.len(), 36, "{id}");
}
