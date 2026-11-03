//! Middleware: global stack, aliases, groups, parameters, priority and terminable middleware.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use oxide_foundation::{
    Application, BootError, Middleware, MiddlewareFuture, MwNext, Outcome, Scope, middleware_fn,
    route_middleware, route_middleware_without,
};
use oxide_http::__private::axum::Router;
use oxide_http::__private::axum::extract::Request;
use oxide_http::__private::axum::http::HeaderValue;
use oxide_http::__private::axum::response::{IntoResponse, Response};
use oxide_http::__private::axum::routing::get;
use oxide_http::OxideHttpError;
use oxide_http::testing::TestClient;
use serde_json::json;

type Log = Arc<Mutex<Vec<String>>>;

fn entries(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

/// Records `<name> before` and `<name> after` around the rest of the chain.
struct Recorder {
    name: &'static str,
    log: Log,
    priority: i32,
}

impl Recorder {
    fn new(name: &'static str, log: &Log) -> Self {
        Self {
            name,
            log: Arc::clone(log),
            priority: 0,
        }
    }

    fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

impl Middleware for Recorder {
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(async move {
            let suffix = if params.is_empty() {
                String::new()
            } else {
                format!("({})", params.join("+"))
            };
            self.log
                .lock()
                .unwrap()
                .push(format!("{}{suffix} before", self.name));
            let response = next.run(request).await;
            self.log
                .lock()
                .unwrap()
                .push(format!("{} after", self.name));
            response
        })
    }

    fn priority(&self) -> i32 {
        self.priority
    }
}

async fn hello() -> &'static str {
    "hello"
}

#[tokio::test]
async fn global_middleware_wraps_everything_in_order_and_sees_unmatched_requests() {
    let log: Log = Arc::default();
    let app = Application::builder()
        .middleware(Recorder::new("a", &log))
        .middleware(Recorder::new("b", &log))
        .routes(Router::new().route("/hello", get(hello)))
        .build()
        .await
        .unwrap();
    let client = app.test_client();

    assert_eq!(client.get("/hello").send().await.text(), "hello");
    assert_eq!(
        entries(&log),
        ["a before", "b before", "b after", "a after"]
    );

    log.lock().unwrap().clear();
    client.get("/nowhere").send().await.assert_not_found();
    assert_eq!(
        entries(&log),
        ["a before", "b before", "b after", "a after"],
        "even a 404 passes through"
    );
}

#[tokio::test]
async fn higher_priority_runs_first_whatever_the_registration_order() {
    let log: Log = Arc::default();
    let app = Application::builder()
        .middleware(Recorder::new("normal", &log))
        .middleware(Recorder::new("urgent", &log).priority(10))
        .middleware(Recorder::new("late", &log).priority(-5))
        .middleware(Recorder::new("normal2", &log))
        .routes(Router::new().route("/", get(hello)))
        .build()
        .await
        .unwrap();
    app.test_client().get("/").send().await.assert_ok();
    let before: Vec<String> = entries(&log)
        .into_iter()
        .filter(|e| e.ends_with("before"))
        .collect();
    assert_eq!(
        before,
        [
            "urgent before",
            "normal before",
            "normal2 before",
            "late before"
        ]
    );
}

fn api_routes() -> Router {
    Router::new()
        .route("/open", get(hello))
        .merge(
            Router::new()
                .route("/guarded", get(hello))
                .layer(route_middleware(&["role:editor,admin", "tag"])),
        )
        .merge(
            Router::new()
                .route("/api", get(hello))
                .layer(route_middleware(&["api"])),
        )
        .merge(
            Router::new()
                .route("/api-quiet", get(hello))
                .layer(route_middleware_without(&["api"], &["tag"])),
        )
}

#[tokio::test]
async fn aliases_take_parameters_and_apply_only_to_their_routes() {
    let log: Log = Arc::default();
    let app = Application::builder()
        .alias("role", Recorder::new("role", &log))
        .alias("tag", Recorder::new("tag", &log))
        .middleware_group("api", ["role:api", "tag"])
        .routes(api_routes())
        .build()
        .await
        .unwrap();
    let client = app.test_client();

    client.get("/open").send().await.assert_ok();
    assert!(
        entries(&log).is_empty(),
        "routes outside the block are untouched"
    );

    client.get("/guarded").send().await.assert_ok();
    assert_eq!(
        entries(&log),
        [
            "role(editor+admin) before",
            "tag before",
            "tag after",
            "role after"
        ]
    );

    log.lock().unwrap().clear();
    client.get("/api").send().await.assert_ok();
    assert_eq!(
        entries(&log),
        ["role(api) before", "tag before", "tag after", "role after"]
    );

    log.lock().unwrap().clear();
    client.get("/api-quiet").send().await.assert_ok();
    assert_eq!(
        entries(&log),
        ["role(api) before", "role after"],
        "`without` drops one member of a group"
    );
}

#[tokio::test]
async fn a_middleware_can_stop_the_request_and_use_services() {
    // Resolves a service from the request scope and answers 403 for anyone but the key holder.
    let gate = middleware_fn(
        |request: Request, next: MwNext, params: Vec<String>| async move {
            let key = request
                .extensions()
                .get::<Scope>()
                .and_then(|scope| scope.resolve::<String>().ok())
                .map(|key| (*key).clone());
            let sent = request
                .headers()
                .get("x-key")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            if sent.is_some() && sent == key {
                let mut response = next.run(request).await;
                response
                    .headers_mut()
                    .insert("x-gate", HeaderValue::from_str(&params.join(",")).unwrap());
                response
            } else {
                OxideHttpError::Forbidden("Wrong key.".into()).into_response()
            }
        },
    );
    let app = Application::builder()
        .bind(|bindings| {
            bindings.singleton(String::from("s3cret"));
        })
        .alias("gate", gate)
        .routes(
            Router::new()
                .route("/vault", get(hello))
                .layer(route_middleware(&["gate:vault"])),
        )
        .build()
        .await
        .unwrap();
    let client = app.test_client();

    let denied = client.get("/vault").send().await;
    assert_eq!(denied.status().as_u16(), 403);
    assert_eq!(denied.json(), json!({ "message": "Wrong key." }));
    assert!(
        denied.headers().get("x-request-id").is_some(),
        "errors from middleware keep the request id"
    );

    let allowed = client.get("/vault").header("X-Key", "s3cret").send().await;
    allowed.assert_ok();
    assert_eq!(allowed.headers().get("x-gate").unwrap(), "vault");
}

#[tokio::test]
async fn misconfigured_middleware_fails_at_build_or_opaquely_at_request_time() {
    let log: Log = Arc::default();
    let unknown_member = Application::builder()
        .middleware_group("api", ["throttle"])
        .build()
        .await
        .unwrap_err();
    assert!(
        matches!(&unknown_member, BootError::Middleware(m) if m.contains("unknown middleware `throttle`")),
        "{unknown_member}"
    );

    let looped = Application::builder()
        .alias("x", Recorder::new("x", &log))
        .middleware_group("a", ["b"])
        .middleware_group("b", ["a"])
        .build()
        .await
        .unwrap_err();
    assert!(
        matches!(&looped, BootError::Middleware(m) if m.contains("loop")),
        "{looped}"
    );

    let clash = Application::builder()
        .alias("api", Recorder::new("api", &log))
        .middleware_group("api", ["api"])
        .build()
        .await
        .unwrap_err();
    assert!(
        matches!(&clash, BootError::Middleware(m) if m.contains("both")),
        "{clash}"
    );

    // A route naming something nobody registered is a 500 that doesn't say why.
    let app = Application::builder()
        .routes(
            Router::new()
                .route("/typo", get(hello))
                .layer(route_middleware(&["authh"])),
        )
        .build()
        .await
        .unwrap();
    let response = app.test_client().get("/typo").send().await;
    assert_eq!(response.status().as_u16(), 500);
    assert!(!response.text().contains("authh"), "{}", response.text());

    // Route middleware needs an Application to supply the registry.
    let bare = Router::new()
        .route("/typo", get(hello))
        .layer(route_middleware(&["x"]));
    assert_eq!(
        TestClient::new(bare)
            .get("/typo")
            .send()
            .await
            .status()
            .as_u16(),
        500
    );
}

struct Auditor {
    outcomes: Arc<Mutex<Vec<Outcome>>>,
}

impl Middleware for Auditor {
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        _params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(async move { next.run(request).await })
    }

    fn terminable(&self) -> bool {
        true
    }

    fn terminate<'a>(
        &'a self,
        outcome: &'a Outcome,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.outcomes.lock().unwrap().push(outcome.clone());
        })
    }
}

#[tokio::test]
async fn terminable_middleware_runs_after_the_response() {
    let outcomes: Arc<Mutex<Vec<Outcome>>> = Arc::default();
    let log: Log = Arc::default();
    let app = Application::builder()
        .middleware(Auditor {
            outcomes: Arc::clone(&outcomes),
        })
        .middleware(Recorder::new("plain", &log))
        .routes(Router::new().route("/hello", get(hello)))
        .build()
        .await
        .unwrap();
    let response = app.test_client().get("/hello?x=1").send().await;
    response.assert_ok();
    assert!(
        outcomes.lock().unwrap().is_empty(),
        "terminate is off the request path"
    );

    for _ in 0..100 {
        if !outcomes.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let outcomes = outcomes.lock().unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(
        (outcomes[0].path.as_str(), outcomes[0].status.as_u16()),
        ("/hello", 200)
    );
    assert_eq!(outcomes[0].method, "GET");
    assert!(outcomes[0].headers.get("content-type").is_some());
}

#[test]
fn debug_output_names_the_remaining_chain() {
    let closure = middleware_fn(
        |request: Request, next: MwNext, _params: Vec<String>| async move {
            let _ = format!("{next:?}");
            let response: Response = next.run(request).await;
            response
        },
    );
    assert!(format!("{closure:?}").contains("FnMiddleware"));
}
