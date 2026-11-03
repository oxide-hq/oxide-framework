//! The exception handler: reporting, rendering, debug gating and domain errors.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use oxide_config::Configuration;
use oxide_foundation::{
    Application, DefaultExceptionHandler, ErrorContext, ExceptionHandler, default_render,
};
use oxide_http::__private::axum::Router;
use oxide_http::__private::axum::http::StatusCode;
use oxide_http::__private::axum::response::{IntoResponse, Response};
use oxide_http::__private::axum::routing::get;
use oxide_http::{DomainError, OxideHttpError, OxideResult};
use oxide_log::{Logger, LoggingConfig, MemoryWriter, tracing};
use serde_json::{Value, json};

async fn broken() -> OxideResult<&'static str> {
    Err(OxideHttpError::server(io::Error::other("database is down")))
}

async fn missing() -> OxideResult<&'static str> {
    Err(OxideHttpError::NotFound("No such post.".into()))
}

async fn hostile() -> OxideResult<&'static str> {
    Err(OxideHttpError::BadRequest(
        "<script>alert('x')</script>".into(),
    ))
}

async fn fine() -> &'static str {
    "ok"
}

#[derive(Debug, thiserror::Error)]
#[error("card declined for order {0}")]
struct PaymentFailed(u32);

impl DomainError for PaymentFailed {
    fn status(&self) -> StatusCode {
        StatusCode::PAYMENT_REQUIRED
    }

    fn message(&self) -> String {
        "Your card was declined.".into()
    }
}

impl From<PaymentFailed> for OxideHttpError {
    fn from(error: PaymentFailed) -> Self {
        Self::domain(error)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("inventory out of sync")]
struct OutOfSync;

impl DomainError for OutOfSync {
    fn report(&self) -> bool {
        true
    }
}

async fn payment() -> OxideResult<&'static str> {
    Err(PaymentFailed(7))?
}

async fn out_of_sync() -> OxideResult<&'static str> {
    Err(OxideHttpError::domain(OutOfSync))
}

fn routes() -> Router {
    Router::new()
        .route("/broken", get(broken))
        .route("/missing", get(missing))
        .route("/hostile", get(hostile))
        .route("/fine", get(fine))
        .route("/payment", get(payment))
        .route("/out-of-sync", get(out_of_sync))
}

async fn app(builder: oxide_foundation::ApplicationBuilder) -> Application {
    builder.routes(routes()).build().await.unwrap()
}

fn config_in(environment: &str) -> Configuration {
    Configuration::builder()
        .vars([("APP_ENV", environment)])
        .load()
        .unwrap()
}

#[derive(Default)]
struct Counting {
    reports: Arc<AtomicU32>,
}

impl ExceptionHandler for Counting {
    fn report(&self, error: &OxideHttpError, _context: &ErrorContext<'_>) {
        assert!(error.status().as_u16() >= 400);
        self.reports.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn only_server_errors_are_reported_by_default() {
    let reports = Arc::new(AtomicU32::new(0));
    let handler = Counting {
        reports: Arc::clone(&reports),
    };
    let app = app(Application::builder().exception_handler(handler)).await;
    let client = app.test_client();

    client.get("/missing").send().await.assert_not_found();
    client.get("/hostile").send().await.assert_bad_request();
    client.get("/fine").send().await.assert_ok();
    assert_eq!(
        reports.load(Ordering::SeqCst),
        0,
        "4xx and 2xx aren't reported"
    );

    assert_eq!(client.get("/broken").send().await.status().as_u16(), 500);
    assert_eq!(reports.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dont_report_silences_a_status() {
    let sink = MemoryWriter::new();
    let config = LoggingConfig::stderr("info");
    let (dispatch, _guard) = Logger::builder(&config)
        .writer("stderr", sink.clone())
        .build()
        .unwrap();
    let _default = tracing::dispatcher::set_default(&dispatch);

    let quiet = app(Application::builder().exception_handler(
        DefaultExceptionHandler::new().dont_report(StatusCode::INTERNAL_SERVER_ERROR),
    ))
    .await;
    quiet.test_client().get("/broken").send().await;
    assert!(sink.lines().is_empty(), "{:?}", sink.lines());

    let loud = app(Application::builder()).await;
    loud.test_client().get("/broken").send().await;
    assert_eq!(sink.lines().len(), 1, "{:?}", sink.lines());
}

#[tokio::test]
async fn reports_carry_the_cause_and_the_request_context() {
    let sink = MemoryWriter::new();
    let config = LoggingConfig::stderr("info");
    let (dispatch, _guard) = Logger::builder(&config)
        .writer("stderr", sink.clone())
        .build()
        .unwrap();
    let _default = tracing::dispatcher::set_default(&dispatch);

    let app = app(Application::builder()).await;
    let response = app
        .test_client()
        .get("/broken")
        .header("X-Request-Id", "req-42")
        .send()
        .await;
    // The client never sees the cause...
    assert_eq!(response.json(), json!({ "message": "Server Error" }));

    // ...but the log line does, with the request id.
    let line = sink.lines().remove(0);
    assert!(
        line.contains("ERROR") && line.contains("request failed"),
        "{line}"
    );
    assert!(
        line.contains("status=500") && line.contains("path=/broken"),
        "{line}"
    );
    assert!(
        line.contains("Server Error <- caused by: database is down"),
        "{line}"
    );
    assert!(line.contains("request_id=req-42"), "{line}");
}

#[tokio::test]
async fn api_clients_keep_the_json_contract_and_browsers_get_html() {
    let app = app(Application::builder()).await;
    let client = app.test_client();

    let json_response = client.get("/missing").send().await;
    assert_eq!(json_response.json(), json!({ "message": "No such post." }));

    let page = client
        .get("/missing")
        .header("Accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .send()
        .await;
    assert_eq!(page.status(), StatusCode::NOT_FOUND);
    page.assert_header("content-type", "text/html; charset=utf-8");
    assert!(page.text().contains("<h1>404</h1>") && page.text().contains("No such post."));
    assert!(
        page.headers().get("x-request-id").is_some(),
        "other headers survive"
    );

    // A client that lists JSON first still gets JSON.
    let api = client
        .get("/missing")
        .header("Accept", "application/json, text/html")
        .send()
        .await;
    assert_eq!(api.json(), json!({ "message": "No such post." }));
}

#[tokio::test]
async fn html_pages_escape_what_the_message_contains() {
    let app = app(Application::builder()).await;
    let page = app
        .test_client()
        .get("/hostile")
        .header("Accept", "text/html")
        .send()
        .await;
    let html = page.text();
    assert!(!html.contains("<script>"), "{html}");
    assert!(
        html.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;"),
        "{html}"
    );
}

#[tokio::test]
async fn debug_detail_appears_only_in_a_local_environment_with_debug_on() {
    async fn chain(builder: oxide_foundation::ApplicationBuilder) -> Value {
        app(builder)
            .await
            .test_client()
            .get("/broken")
            .send()
            .await
            .json()
    }

    let local = chain(
        Application::builder()
            .config(config_in("local"))
            .debug(true),
    )
    .await;
    assert_eq!(local["message"], "Server Error");
    assert_eq!(
        local["exception"]["chain"],
        json!(["Server Error", "database is down"])
    );

    // Each of these must leak nothing.
    for body in [
        chain(
            Application::builder()
                .config(config_in("production"))
                .debug(true),
        )
        .await,
        chain(
            Application::builder()
                .config(config_in("staging"))
                .debug(true),
        )
        .await,
        chain(Application::builder().debug(true)).await,
        chain(Application::builder().config(config_in("local"))).await,
    ] {
        assert_eq!(body, json!({ "message": "Server Error" }), "{body}");
    }

    let page = app(Application::builder()
        .config(config_in("local"))
        .debug(true))
    .await
    .test_client()
    .get("/broken")
    .header("Accept", "text/html")
    .send()
    .await
    .text();
    assert!(
        page.contains("<pre>Server Error\n  caused by: database is down</pre>"),
        "{page}"
    );
}

#[tokio::test]
async fn domain_errors_choose_their_status_message_and_reporting() {
    let reports = Arc::new(AtomicU32::new(0));
    let app = app(Application::builder().exception_handler(Counting {
        reports: Arc::clone(&reports),
    }))
    .await;
    let client = app.test_client();

    let response = client.get("/payment").send().await;
    assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
    assert_eq!(
        response.json(),
        json!({ "message": "Your card was declined." })
    );
    assert_eq!(
        reports.load(Ordering::SeqCst),
        0,
        "a 402 isn't a server error"
    );

    // `report()` can opt a domain error in, whatever its status.
    assert_eq!(
        client.get("/out-of-sync").send().await.status().as_u16(),
        500
    );
    assert_eq!(reports.load(Ordering::SeqCst), 1);
    assert_eq!(
        OxideHttpError::domain(PaymentFailed(1)).to_string(),
        "card declined for order 1"
    );
}

struct Teapot;

impl ExceptionHandler for Teapot {
    fn should_report(&self, _error: &OxideHttpError) -> bool {
        false
    }

    fn render(
        &self,
        error: &OxideHttpError,
        context: &ErrorContext<'_>,
        response: Response,
    ) -> Response {
        if error.status() == StatusCode::NOT_FOUND {
            return (StatusCode::IM_A_TEAPOT, "short and stout").into_response();
        }
        default_render(error, context, response)
    }
}

#[tokio::test]
async fn a_custom_handler_can_replace_a_response_or_delegate() {
    let app = app(Application::builder().exception_handler(Teapot)).await;
    let client = app.test_client();
    let replaced = client.get("/missing").send().await;
    assert_eq!(replaced.status(), StatusCode::IM_A_TEAPOT);
    assert_eq!(replaced.text(), "short and stout");
    // Delegating to the default keeps everything else as it was.
    assert_eq!(
        client.get("/hostile").send().await.json(),
        json!({ "message": "<script>alert('x')</script>" })
    );
}
