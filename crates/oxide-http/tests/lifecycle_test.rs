//! End-to-end test of the Phase 1 HTTP lifecycle:
//! service injection → route-model binding → form request (parse, authorize, validate) → response.

// The mocks resolve synchronously, but `async fn` in trait impls is exactly the ergonomics this
// suite demonstrates (real implementations would await a database).
#![allow(clippy::unused_async_trait_impl)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::http::request::Parts;
use axum::routing::{get, put};
use axum::{Json, Router};
use oxide_http::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

// --- Services -------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Service)]
struct MockMailer {
    sent: Arc<Mutex<Vec<String>>>,
}

impl MockMailer {
    fn send(&self, message: impl Into<String>) {
        self.sent.lock().unwrap().push(message.into());
    }

    fn sent(&self) -> Vec<String> {
        self.sent.lock().unwrap().clone()
    }
}

#[derive(Clone, Debug, Default, Service)]
struct MockDb {
    posts: Arc<Mutex<HashMap<u64, Post>>>,
}

#[derive(Clone, Debug, ServiceContainer)]
struct TestContext {
    mailer: MockMailer,
    db: MockDb,
}

// --- Model ----------------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Post {
    id: u64,
    title: String,
    body: String,
}

impl RouteBindable<TestContext> for Post {
    type Id = u64;

    fn param_name() -> &'static str {
        "post"
    }

    async fn bind_from_state(id: u64, state: &TestContext) -> Result<Option<Self>, OxideHttpError> {
        Ok(state.db.posts.lock().unwrap().get(&id).cloned())
    }
}

// --- Form request ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Validate, FormRequest)]
#[form_request(authorize)]
struct UpdatePostRequest {
    #[validate(length(min = 5, message = "Title must be at least 5 characters"))]
    title: String,
    #[validate(length(max = 1000, message = "Body may not be greater than 1000 characters"))]
    body: Option<String>,
}

impl Authorize<TestContext> for UpdatePostRequest {
    async fn authorize(&self, parts: &Parts, _state: &TestContext) -> bool {
        parts
            .headers
            .get("x-user-role")
            .is_some_and(|role| role == "editor")
    }
}

// --- Handlers -------------------------------------------------------------------------------

async fn update_post(
    mailer: MockMailer,
    Bind(mut post): Bind<Post>,
    Validated(req): Validated<UpdatePostRequest>,
) -> Json<Post> {
    post.title = req.title;
    if let Some(body) = req.body {
        post.body = body;
    }
    mailer.send(format!("Post {} updated: {}", post.id, post.title));
    Json(post)
}

/// Uses the task-local facade instead of an injected argument.
async fn facade_probe() -> Json<serde_json::Value> {
    let db = TestContext::resolve::<MockDb>();
    let count = db.posts.lock().unwrap().len();
    Json(json!({ "posts": count }))
}

// --- Fixtures -------------------------------------------------------------------------------

fn context() -> TestContext {
    let db = MockDb::default();
    db.posts.lock().unwrap().insert(
        1,
        Post {
            id: 1,
            title: "Original title".into(),
            body: "Original body".into(),
        },
    );
    TestContext {
        mailer: MockMailer::default(),
        db,
    }
}

fn app(ctx: &TestContext) -> TestClient {
    let router = Router::new()
        .route("/posts/{post}", put(update_post))
        .route("/facade", get(facade_probe))
        .layer(axum::middleware::from_fn_with_state(
            ctx.clone(),
            facade_middleware::<TestContext>,
        ))
        .with_state(ctx.clone());
    TestClient::new(router)
}

// --- Tests ----------------------------------------------------------------------------------

#[tokio::test]
async fn missing_model_returns_404() {
    let ctx = context();
    app(&ctx)
        .put("/posts/999")
        .header("X-User-Role", "editor")
        .json(&json!({ "title": "A valid title" }))
        .send()
        .await
        .assert_not_found()
        .assert_json_path("message", "No query results for model [post] 999");

    assert_eq!(ctx.mailer.sent(), Vec::<String>::new());
}

#[tokio::test]
async fn unparseable_route_key_returns_404() {
    let ctx = context();
    app(&ctx)
        .put("/posts/not-a-number")
        .header("X-User-Role", "editor")
        .json(&json!({ "title": "A valid title" }))
        .send()
        .await
        .assert_not_found();
}

#[tokio::test]
async fn unauthorized_request_returns_403() {
    let ctx = context();
    app(&ctx)
        .put("/posts/1")
        .header("X-User-Role", "viewer")
        .json(&json!({ "title": "A valid title" }))
        .send()
        .await
        .assert_forbidden()
        .assert_json_path("message", "This action is unauthorized.");

    assert_eq!(ctx.mailer.sent(), Vec::<String>::new());
}

#[tokio::test]
async fn invalid_payload_returns_422_with_laravel_errors() {
    let ctx = context();
    app(&ctx)
        .put("/posts/1")
        .header("X-User-Role", "editor")
        .json(&json!({ "title": "abc" }))
        .send()
        .await
        .assert_unprocessable()
        .assert_validation_error_for("title")
        .assert_json_path("message", "The given data was invalid.")
        .assert_json_path(
            "errors.title",
            json!(["Title must be at least 5 characters"]),
        );

    assert_eq!(ctx.mailer.sent(), Vec::<String>::new());
}

#[tokio::test]
async fn valid_request_updates_post_and_sends_notification() {
    let ctx = context();
    let response = app(&ctx)
        .put("/posts/1")
        .header("X-User-Role", "editor")
        .json(&json!({ "title": "A brand new title", "body": "Fresh body" }))
        .send()
        .await;

    response
        .assert_ok()
        .assert_json_path("id", 1)
        .assert_json_path("/title", "A brand new title")
        .assert_json_path("body", "Fresh body");

    assert_eq!(
        response.json_as::<Post>(),
        Post {
            id: 1,
            title: "A brand new title".into(),
            body: "Fresh body".into(),
        }
    );
    assert_eq!(
        ctx.mailer.sent(),
        vec!["Post 1 updated: A brand new title".to_string()]
    );
}

#[tokio::test]
async fn form_urlencoded_payload_is_supported() {
    let ctx = context();
    app(&ctx)
        .put("/posts/1")
        .header("X-User-Role", "editor")
        .form(&[("title", "Form submitted title")])
        .send()
        .await
        .assert_ok()
        .assert_json_path("title", "Form submitted title")
        .assert_json_path("body", "Original body");

    assert_eq!(ctx.mailer.sent().len(), 1);
}

#[tokio::test]
async fn malformed_json_returns_400() {
    let ctx = context();
    app(&ctx)
        .put("/posts/1")
        .header("X-User-Role", "editor")
        .header("Content-Type", "application/json")
        .body("{ not json")
        .send()
        .await
        .assert_bad_request();
}

#[tokio::test]
async fn facade_resolves_services_inside_request_scope() {
    let ctx = context();
    app(&ctx)
        .get("/facade")
        .send()
        .await
        .assert_ok()
        .assert_json_path("posts", 1);

    // Outside of a request scope there is no container.
    assert!(TestContext::try_current().is_none());
}
