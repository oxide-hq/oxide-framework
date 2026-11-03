//! The built-in middleware, each through a real `Application`.

use oxide_config::Configuration;
use oxide_foundation::{
    Application, ConvertEmptyStringsToNull, CorsConfig, HandleCors, SecurityHeaders,
    SetCacheHeaders, TrimStrings, TrustHosts, ValidatePostSize,
};
use oxide_http::__private::axum::Json;
use oxide_http::__private::axum::Router;
use oxide_http::__private::axum::http::header::{HeaderName, HeaderValue};
use oxide_http::__private::axum::response::IntoResponse;
use oxide_http::__private::axum::routing::{get, post};
use oxide_http::testing::{TestClient, TestResponse};
use serde_json::{Value, json};

async fn hello() -> &'static str {
    "hello"
}

async fn echo_json(Json(body): Json<Value>) -> Json<Value> {
    Json(body)
}

async fn echo_text(body: String) -> String {
    body
}

async fn custom_cache() -> impl IntoResponse {
    ([("cache-control", "no-cache")], "x")
}

async fn custom_frame() -> impl IntoResponse {
    ([("x-frame-options", "DENY")], "x")
}

fn routes() -> Router {
    Router::new()
        .route("/hello", get(hello).post(hello))
        .route("/echo", post(echo_json))
        .route("/text", post(echo_text))
        .route("/cache", get(custom_cache))
        .route("/frame", get(custom_frame))
}

async fn client(builder: oxide_foundation::ApplicationBuilder) -> TestClient {
    builder
        .routes(routes())
        .build()
        .await
        .unwrap()
        .test_client()
}

fn header<'a>(response: &'a TestResponse, name: &str) -> Option<&'a str> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn security_headers_are_added_but_never_override_the_handler() {
    let client = client(
        Application::builder().middleware(
            SecurityHeaders::new()
                .hsts(31_536_000, true)
                .content_security_policy("default-src 'self'")
                .unwrap()
                .header(
                    HeaderName::from_static("x-custom"),
                    HeaderValue::from_static("1"),
                ),
        ),
    )
    .await;

    let response = client.get("/hello").send().await;
    assert_eq!(header(&response, "x-content-type-options"), Some("nosniff"));
    assert_eq!(header(&response, "x-frame-options"), Some("SAMEORIGIN"));
    assert_eq!(
        header(&response, "referrer-policy"),
        Some("strict-origin-when-cross-origin")
    );
    assert_eq!(
        header(&response, "strict-transport-security"),
        Some("max-age=31536000; includeSubDomains")
    );
    assert_eq!(
        header(&response, "content-security-policy"),
        Some("default-src 'self'")
    );
    assert_eq!(header(&response, "x-custom"), Some("1"));

    // Even a 404 carries them, and a header the handler set is kept.
    assert_eq!(
        header(&client.get("/nope").send().await, "x-content-type-options"),
        Some("nosniff")
    );
    assert_eq!(
        header(&client.get("/frame").send().await, "x-frame-options"),
        Some("DENY")
    );
    assert!(
        SecurityHeaders::new()
            .content_security_policy("bad\nvalue")
            .is_err()
    );
}

#[tokio::test]
async fn post_size_is_limited_by_declared_length_and_while_reading() {
    let client = client(Application::builder().middleware(ValidatePostSize::new(16))).await;

    let declared = client
        .post("/text")
        .header("Content-Length", "999")
        .body("x")
        .send()
        .await;
    assert_eq!(declared.status().as_u16(), 413);
    assert_eq!(declared.json(), json!({ "message": "Payload Too Large" }));

    client
        .post("/text")
        .body("sixteen bytes!!!")
        .send()
        .await
        .assert_ok();

    // No declared length (chunked): the body itself is cut off while it is read.
    let streamed = client
        .post("/text")
        .body("far more than sixteen bytes")
        .send()
        .await;
    assert_eq!(streamed.status().as_u16(), 413);
}

#[tokio::test]
async fn cache_headers_apply_only_to_successful_reads() {
    let client = client(
        Application::builder()
            .middleware(SetCacheHeaders::new().public().max_age(3600).immutable()),
    )
    .await;
    assert_eq!(
        header(&client.get("/hello").send().await, "cache-control"),
        Some("public, max-age=3600, immutable")
    );
    assert_eq!(
        header(&client.post("/hello").send().await, "cache-control"),
        None,
        "writes aren't cached"
    );
    assert_eq!(
        header(&client.get("/nope").send().await, "cache-control"),
        None,
        "errors aren't cached"
    );
    assert_eq!(
        header(&client.get("/cache").send().await, "cache-control"),
        Some("no-cache"),
        "the handler wins"
    );

    let no_store = self::client(
        Application::builder().middleware(
            SetCacheHeaders::new()
                .no_store()
                .must_revalidate()
                .private()
                .s_maxage(1)
                .no_cache(),
        ),
    )
    .await;
    assert_eq!(
        header(&no_store.get("/hello").send().await, "cache-control"),
        Some("no-store, must-revalidate, private, s-maxage=1, no-cache")
    );
    let empty = self::client(Application::builder().middleware(SetCacheHeaders::new())).await;
    assert_eq!(
        header(&empty.get("/hello").send().await, "cache-control"),
        None
    );
}

#[tokio::test]
async fn untrusted_hosts_are_refused() {
    let client = client(
        Application::builder().middleware(TrustHosts::new(["example.com", "*.example.org"])),
    )
    .await;
    for host in ["example.com", "Example.com:8080", "api.example.org"] {
        client
            .get("/hello")
            .header("Host", host)
            .send()
            .await
            .assert_ok();
    }
    for host in ["evil.net", "example.org", "example.com.evil.net"] {
        let response = client.get("/hello").header("Host", host).send().await;
        assert_eq!(response.status().as_u16(), 400, "{host}");
        assert_eq!(response.json(), json!({ "message": "Invalid host." }));
    }
    assert_eq!(
        client.get("/hello").send().await.status().as_u16(),
        400,
        "no host at all"
    );
}

#[tokio::test]
async fn json_strings_are_trimmed_and_empty_ones_become_null() {
    let client = client(
        Application::builder()
            .middleware(TrimStrings::new().except(["bio"]))
            .middleware(ConvertEmptyStringsToNull::new().except(["keep_empty"])),
    )
    .await;
    let sent = json!({
        "name": "  Ann Lee  ",
        "password": "  keep me  ",
        "bio": "  untouched  ",
        "blank": "   ",
        "empty": "",
        "keep_empty": "",
        "tags": [" a ", ""],
        "nested": { "city": " Lisbon " },
        "age": 7,
    });
    let echoed = client.post("/echo").json(&sent).send().await.json();
    assert_eq!(
        echoed,
        json!({
            "name": "Ann Lee",
            "password": "  keep me  ",
            "bio": "  untouched  ",
            "blank": null,
            "empty": null,
            "keep_empty": "",
            "tags": ["a", null],
            "nested": { "city": "Lisbon" },
            "age": 7,
        })
    );
}

#[tokio::test]
async fn only_json_bodies_are_rewritten_and_bad_ones_reach_the_handler() {
    let client = client(
        Application::builder()
            .middleware(TrimStrings::new())
            .middleware(ConvertEmptyStringsToNull::new()),
    )
    .await;

    // Plain text is left exactly as sent.
    assert_eq!(
        client
            .post("/text")
            .header("Content-Type", "text/plain")
            .body("  spaced  ")
            .send()
            .await
            .text(),
        "  spaced  "
    );
    // Broken JSON isn't swallowed: the handler's own extractor answers.
    let broken = client
        .post("/echo")
        .header("Content-Type", "application/json")
        .body("{ nope")
        .send()
        .await;
    assert!(broken.status().is_client_error(), "{}", broken.status());
    // `+json` media types and charset parameters count as JSON.
    let patch = client
        .post("/echo")
        .header("Content-Type", "application/json; charset=utf-8")
        .body(r#"{"a":" x "}"#)
        .send()
        .await;
    assert_eq!(patch.json(), json!({ "a": "x" }));
}

#[tokio::test]
async fn oversized_json_bodies_are_refused() {
    let client = client(Application::builder().middleware(TrimStrings::new().max_body(20))).await;
    let big = client
        .post("/echo")
        .json(&json!({ "name": "a long enough value to pass twenty bytes" }))
        .send()
        .await;
    assert_eq!(big.status().as_u16(), 413);
    client
        .post("/echo")
        .json(&json!({ "a": "b" }))
        .send()
        .await
        .assert_ok();
}

fn cors(origins: &[&str], credentials: bool) -> CorsConfig {
    CorsConfig {
        allowed_origins: origins.iter().map(ToString::to_string).collect(),
        allowed_methods: vec!["GET".into(), "POST".into()],
        allowed_headers: vec!["X-Token".into(), "Content-Type".into()],
        exposed_headers: vec!["X-Request-Id".into()],
        max_age: 600,
        supports_credentials: credentials,
    }
}

#[tokio::test]
async fn cors_answers_preflights_and_marks_allowed_origins() {
    let client = client(
        Application::builder()
            .middleware(HandleCors::new(&cors(&["https://app.test"], true)).unwrap()),
    )
    .await;

    let preflight = client
        .options("/hello")
        .header("Origin", "https://app.test")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await;
    assert_eq!(preflight.status().as_u16(), 204);
    assert_eq!(
        header(&preflight, "access-control-allow-origin"),
        Some("https://app.test")
    );
    assert_eq!(
        header(&preflight, "access-control-allow-credentials"),
        Some("true")
    );
    assert_eq!(
        header(&preflight, "access-control-allow-methods"),
        Some("GET, POST")
    );
    assert_eq!(
        header(&preflight, "access-control-allow-headers"),
        Some("X-Token, Content-Type")
    );
    assert_eq!(header(&preflight, "access-control-max-age"), Some("600"));
    assert!(header(&preflight, "vary").unwrap().contains("Origin"));

    let allowed = client
        .get("/hello")
        .header("Origin", "https://app.test")
        .send()
        .await;
    allowed.assert_ok();
    assert_eq!(
        header(&allowed, "access-control-allow-origin"),
        Some("https://app.test")
    );
    assert_eq!(
        header(&allowed, "access-control-expose-headers"),
        Some("X-Request-Id")
    );
    assert!(header(&allowed, "vary").unwrap().contains("Origin"));

    // A stranger's origin still gets an answer, but nothing that lets the browser read it.
    let stranger = client
        .get("/hello")
        .header("Origin", "https://evil.test")
        .send()
        .await;
    assert_eq!(header(&stranger, "access-control-allow-origin"), None);
    let denied = client
        .options("/hello")
        .header("Origin", "https://evil.test")
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await;
    assert_eq!(header(&denied, "access-control-allow-origin"), None);
    assert_eq!(header(&denied, "access-control-allow-methods"), None);

    // Same-origin requests (no Origin header) are untouched.
    assert_eq!(
        header(
            &client.get("/hello").send().await,
            "access-control-allow-origin"
        ),
        None
    );
}

#[tokio::test]
async fn cors_wildcards_and_misconfiguration() {
    let wildcard =
        client(Application::builder().middleware(HandleCors::new(&cors(&["*"], false)).unwrap()))
            .await;
    let response = wildcard
        .get("/hello")
        .header("Origin", "https://anywhere.test")
        .send()
        .await;
    assert_eq!(header(&response, "access-control-allow-origin"), Some("*"));
    assert_eq!(header(&response, "access-control-allow-credentials"), None);

    // `*` for headers echoes what the browser asked for.
    let mut config = cors(&["*"], false);
    config.allowed_headers = vec!["*".into()];
    let echo = client(Application::builder().middleware(HandleCors::new(&config).unwrap())).await;
    let preflight = echo
        .options("/hello")
        .header("Origin", "https://a.test")
        .header("Access-Control-Request-Method", "GET")
        .header("Access-Control-Request-Headers", "x-one, x-two")
        .send()
        .await;
    assert_eq!(
        header(&preflight, "access-control-allow-headers"),
        Some("x-one, x-two")
    );

    // A wildcard origin with credentials would let any site act as the user.
    let error = HandleCors::new(&cors(&["*"], true)).unwrap_err();
    assert!(error.contains("supports_credentials"), "{error}");
    assert!(
        HandleCors::new(&CorsConfig {
            allowed_methods: vec!["bad\nvalue".into()],
            ..cors(&["*"], false)
        })
        .is_err()
    );
}

#[test]
fn cors_config_has_sensible_defaults() {
    let config = Configuration::builder()
        .vars(Vec::<(String, String)>::new())
        .section::<CorsConfig>()
        .load()
        .unwrap();
    let cors = config.get::<CorsConfig>().unwrap();
    assert_eq!(cors.allowed_origins, ["*"]);
    assert_eq!(
        cors.allowed_methods,
        ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
    );
    assert!(!cors.supports_credentials && cors.max_age == 0 && cors.exposed_headers.is_empty());
    assert!(HandleCors::new(&cors).is_ok());
}

#[tokio::test]
async fn unchanged_bodies_reach_the_handler_byte_for_byte() {
    let client = client(
        Application::builder()
            .middleware(TrimStrings::new())
            .middleware(ConvertEmptyStringsToNull::new()),
    )
    .await;
    // Key order, spacing and number formatting survive when nothing needs rewriting.
    let body = r#"{ "z":1.50, "a":[1,2],"n":12345678901234567890123, "s":"ok" }"#;
    let echoed = client
        .post("/text")
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await;
    assert_eq!(echoed.text(), body);
    // A chunked body that does change loses its Transfer-Encoding for a real Content-Length.
    let changed = client
        .post("/text")
        .header("Content-Type", "application/json")
        .header("Transfer-Encoding", "chunked")
        .body(r#"{"a":" x "}"#)
        .send()
        .await;
    assert_eq!(changed.text(), r#"{"a":"x"}"#);
}

async fn sets_cookie() -> impl IntoResponse {
    ([("set-cookie", "session=1")], "x")
}

#[tokio::test]
async fn personal_and_unrelated_responses_are_handled_safely() {
    let routes = routes().route("/login", get(sets_cookie));
    let app = Application::builder()
        .middleware(SetCacheHeaders::new().public().max_age(60))
        .middleware(HandleCors::new(&cors(&["https://app.test"], false)).unwrap())
        .routes(routes)
        .build()
        .await
        .unwrap();
    let client = app.test_client();
    let login = client.get("/login").send().await;
    assert_eq!(
        header(&login, "cache-control"),
        None,
        "a response that sets a cookie is never made public"
    );
    // Even without an Origin header the response varies on it, for the caches' sake.
    let plain = client.get("/hello").send().await;
    assert!(header(&plain, "vary").unwrap().contains("Origin"));
    assert_eq!(header(&plain, "cache-control"), Some("public, max-age=60"));
}
