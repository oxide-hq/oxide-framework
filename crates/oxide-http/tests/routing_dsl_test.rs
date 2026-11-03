//! Phase 2: the `routes!` DSL, resource controllers, named routes and URL generation.

// Handlers and bindings use `async fn` for the ergonomics under test even when the in-memory
// store makes them synchronous.
#![allow(clippy::unused_async, clippy::unused_async_trait_impl)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::{Path, Request};
use axum::http::header::LOCATION;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use oxide_http::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

// --- State ----------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Service)]
struct PostStore {
    inner: Arc<Mutex<BTreeMap<u64, Post>>>,
}

impl PostStore {
    fn insert(&self, title: String, body: String) -> Post {
        let mut posts = self.inner.lock().unwrap();
        let id = posts.keys().next_back().map_or(1, |id| id + 1);
        let post = Post {
            id,
            title,
            body,
            published: false,
        };
        posts.insert(id, post.clone());
        post
    }

    fn save(&self, post: &Post) {
        self.inner.lock().unwrap().insert(post.id, post.clone());
    }

    fn remove(&self, id: u64) {
        self.inner.lock().unwrap().remove(&id);
    }

    fn all(&self) -> Vec<Post> {
        self.inner.lock().unwrap().values().cloned().collect()
    }
}

#[derive(Clone, Debug, Default, ServiceContainer)]
struct AppContext {
    posts: PostStore,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Post {
    id: u64,
    title: String,
    body: String,
    published: bool,
}

impl RouteBindable<AppContext> for Post {
    type Id = u64;

    fn param_name() -> &'static str {
        "post"
    }

    async fn bind_from_state(id: u64, ctx: &AppContext) -> Result<Option<Self>, OxideHttpError> {
        Ok(ctx.posts.inner.lock().unwrap().get(&id).cloned())
    }
}

#[derive(Debug, Deserialize, Validate, FormRequest)]
struct CreatePostRequest {
    #[validate(length(min = 5, message = "Title must be at least 5 characters"))]
    title: String,
    #[validate(length(min = 1, message = "Body is required"))]
    body: String,
}

// --- Controllers ----------------------------------------------------------------------------

struct PostController;

impl PostController {
    async fn index(posts: PostStore) -> Json<Vec<Post>> {
        Json(posts.all())
    }

    async fn store(
        posts: PostStore,
        urls: UrlGenerator,
        Validated(req): Validated<CreatePostRequest>,
    ) -> Result<Response, OxideHttpError> {
        let post = posts.insert(req.title, req.body);
        let location = urls.url("posts.show", &[("post", &post.id.to_string())])?;
        let mut response = (StatusCode::CREATED, Json(post)).into_response();
        response.headers_mut().insert(
            LOCATION,
            HeaderValue::try_from(location).map_err(|e| OxideHttpError::Internal(e.to_string()))?,
        );
        Ok(response)
    }

    async fn show(Bind(post): Bind<Post>) -> Json<Post> {
        Json(post)
    }

    async fn update(
        posts: PostStore,
        Bind(mut post): Bind<Post>,
        Validated(req): Validated<CreatePostRequest>,
    ) -> Json<Post> {
        post.title = req.title;
        post.body = req.body;
        posts.save(&post);
        Json(post)
    }

    async fn destroy(posts: PostStore, Bind(post): Bind<Post>) -> StatusCode {
        posts.remove(post.id);
        StatusCode::NO_CONTENT
    }

    async fn publish(posts: PostStore, Bind(mut post): Bind<Post>) -> Json<Post> {
        post.published = true;
        posts.save(&post);
        Json(post)
    }
}

struct CategoryController;

impl CategoryController {
    async fn index() -> Json<Vec<&'static str>> {
        Json(vec!["rust", "web"])
    }

    async fn show(Path(category): Path<String>) -> String {
        format!("category {category}")
    }
}

async fn health() -> &'static str {
    "ok"
}

async fn clear_health() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn home(urls: UrlGenerator) -> String {
    urls.url("posts.index", &[]).unwrap()
}

/// Marks responses that went through the admin middleware block.
async fn tag_admin(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("x-admin", HeaderValue::from_static("true"));
    response
}

// --- Router ---------------------------------------------------------------------------------

fn router() -> OxideRouter<AppContext> {
    routes! {
        get("/", home).name("home");

        prefix("/api/v1") {
            get("/health", health).name("health");

            resource("/posts", PostController);

            post("/posts/{post}/publish", PostController::publish).name("posts.publish");

            middleware(middleware::from_fn(tag_admin)) {
                // Same path as `health` above, different method: the routers must merge.
                delete("/health", clear_health).name("health.clear");
            }

            prefix("/admin").name("admin.") {
                middleware(middleware::from_fn(tag_admin)) {
                    resource("/categories", CategoryController, only = [index, show]);
                }
            }
        }
    }
}

fn app() -> (AppContext, TestClient) {
    let ctx = AppContext::default();
    let client = TestClient::new(router().with_state(ctx.clone()));
    (ctx, client)
}

// --- Tests ----------------------------------------------------------------------------------

#[test]
fn resource_registers_the_five_named_actions() {
    let router = router();
    let registry = router.registry();

    let expect = [
        ("posts.index", Method::GET, "/api/v1/posts"),
        ("posts.store", Method::POST, "/api/v1/posts"),
        ("posts.show", Method::GET, "/api/v1/posts/{post}"),
        ("posts.update", Method::PUT, "/api/v1/posts/{post}"),
        ("posts.destroy", Method::DELETE, "/api/v1/posts/{post}"),
        (
            "posts.publish",
            Method::POST,
            "/api/v1/posts/{post}/publish",
        ),
        (
            "admin.categories.index",
            Method::GET,
            "/api/v1/admin/categories",
        ),
        (
            "admin.categories.show",
            Method::GET,
            "/api/v1/admin/categories/{category}",
        ),
    ];
    for (name, method, path) in expect {
        let route = registry
            .get(name)
            .unwrap_or_else(|| panic!("route `{name}` missing"));
        assert_eq!(
            (&route.method, route.path.as_str()),
            (&method, path),
            "{name}"
        );
    }

    assert_eq!(
        registry.get("posts.show").unwrap().params,
        vec!["post".to_owned()]
    );
    // `only = [index, show]` skips the other actions.
    assert!(registry.get("admin.categories.store").is_none());
    // PATCH is routed to `update` too, without a second name.
    assert!(registry.routes().iter().any(|r| r.method == Method::PATCH
        && r.path == "/api/v1/posts/{post}"
        && r.name.is_none()));
}

#[test]
fn route_macro_generates_urls() {
    let _ = app(); // finalizing a router publishes its names

    assert_eq!(route!("posts.index"), "/api/v1/posts");
    assert_eq!(route!("posts.show", post = 1), "/api/v1/posts/1");
    assert_eq!(route!("posts.publish", post = 1), "/api/v1/posts/1/publish");
    assert_eq!(route!("home"), "/");
    assert_eq!(
        route!("posts.index", page = 2, q = "oxide rs"),
        "/api/v1/posts?page=2&q=oxide%20rs"
    );
    assert_eq!(
        route!("admin.categories.show", category = "rust"),
        "/api/v1/admin/categories/rust"
    );
}

#[test]
fn url_for_fails_fast_on_unknown_routes_and_missing_params() {
    let _ = app();

    let missing = url_for("posts.show", &[]).unwrap_err();
    assert!(
        missing.to_string().contains("missing parameter `post`"),
        "{missing}"
    );

    let unknown = url_for("posts.archive", &[]).unwrap_err();
    assert!(
        unknown
            .to_string()
            .contains("`posts.archive` is not defined"),
        "{unknown}"
    );
}

#[test]
#[should_panic(expected = "missing parameter `post`")]
fn route_macro_panics_on_missing_param() {
    let _ = app();
    let _ = route!("posts.show");
}

#[tokio::test]
async fn resource_actions_handle_the_full_lifecycle() {
    let (ctx, client) = app();

    // store
    let created = client
        .post(route!("posts.index"))
        .json(&json!({ "title": "Hello Oxide", "body": "First post" }))
        .send()
        .await;
    created
        .assert_created()
        .assert_json_path("id", 1)
        .assert_header("location", &route!("posts.show", post = 1));

    // index
    client
        .get(route!("posts.index"))
        .send()
        .await
        .assert_ok()
        .assert_json_path("0.title", "Hello Oxide");

    // show
    client
        .get(route!("posts.show", post = 1))
        .send()
        .await
        .assert_ok()
        .assert_json_path("body", "First post");

    // update (PUT) and PATCH
    client
        .put(route!("posts.update", post = 1))
        .json(&json!({ "title": "Hello again", "body": "Edited" }))
        .send()
        .await
        .assert_ok()
        .assert_json_path("title", "Hello again");
    client
        .patch(route!("posts.update", post = 1))
        .json(&json!({ "title": "Patched title", "body": "Edited" }))
        .send()
        .await
        .assert_ok()
        .assert_json_path("title", "Patched title");

    // custom action
    client
        .post(route!("posts.publish", post = 1))
        .send()
        .await
        .assert_ok()
        .assert_json_path("published", true);
    assert!(ctx.posts.all()[0].published);

    // destroy
    client
        .delete(route!("posts.destroy", post = 1))
        .send()
        .await
        .assert_no_content();
    client
        .get(route!("posts.show", post = 1))
        .send()
        .await
        .assert_not_found();
    assert_eq!(ctx.posts.all(), Vec::<Post>::new());
}

#[tokio::test]
async fn store_validates_the_form_request() {
    let (_, client) = app();
    client
        .post(route!("posts.store"))
        .json(&json!({ "title": "abc", "body": "" }))
        .send()
        .await
        .assert_validation_error_for("title")
        .assert_validation_error_for("body");
}

#[tokio::test]
async fn handlers_can_generate_urls_through_the_extension() {
    let (_, client) = app();
    let response = client.get("/").send().await;
    response.assert_ok();
    assert_eq!(response.text(), "/api/v1/posts");
}

#[tokio::test]
async fn middleware_blocks_only_wrap_their_routes() {
    let (_, client) = app();

    let public = client.get(route!("health")).send().await;
    public.assert_ok();
    assert!(public.headers().get("x-admin").is_none());

    // Same path, different method, registered inside the middleware block.
    client
        .delete(route!("health.clear"))
        .send()
        .await
        .assert_no_content()
        .assert_header("x-admin", "true");

    client
        .get(route!("admin.categories.index"))
        .send()
        .await
        .assert_ok()
        .assert_header("x-admin", "true")
        .assert_json_path("1", "web");

    let show = client
        .get(route!("admin.categories.show", category = "rust"))
        .send()
        .await;
    show.assert_ok();
    assert_eq!(show.text(), "category rust");

    // `only = [index, show]`: no store route.
    client
        .post("/api/v1/admin/categories")
        .json(&json!({}))
        .send()
        .await
        .assert_status(StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn nest_prefix_prefixes_child_route_patterns() {
    let child = OxideRouter::<AppContext>::new()
        .route_named(Method::GET, "/", health, Some("nested.root"))
        .route_named(
            Method::GET,
            "/posts/{post}",
            PostController::show,
            Some("nested.posts.show"),
        );
    let router = OxideRouter::new().nest_prefix("/nested/", child);

    assert_eq!(
        router.registry().get("nested.root").unwrap().path,
        "/nested"
    );
    assert_eq!(
        router
            .registry()
            .url("nested.posts.show", &[("post", "9")])
            .unwrap(),
        "/nested/posts/9"
    );

    let ctx = AppContext::default();
    ctx.posts.insert("Nested post".into(), "Body".into());
    let client = TestClient::new(router.with_state(ctx));
    client.get("/nested").send().await.assert_ok();
    client
        .get("/nested/posts/1")
        .send()
        .await
        .assert_ok()
        .assert_json_path("title", "Nested post");
}
