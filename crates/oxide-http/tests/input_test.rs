//! `Input` and `RequestExt` through real routes.

use axum::Json;
use axum::extract::DefaultBodyLimit;
use axum::http::request::Parts;
use oxide_http::prelude::*;
use serde_json::{Value, json};

async fn echo(input: Input) -> Json<Value> {
    Json(json!({
        "all": input.all(),
        "name": input.string("user.name"),
        "page": input.integer("page"),
        "admin": input.boolean("admin"),
        "filled_note": input.filled("note"),
    }))
}

async fn who(parts: Parts) -> Json<Value> {
    Json(json!({
        "wants_json": parts.wants_json(),
        "bearer": parts.bearer_token(),
        "route": parts.route_name(),
        "posts_route": parts.route_is(&["posts.*"]),
        "api_path": parts.path_is(&["api/*"]),
        "url": parts.full_url(),
        "agent": parts.user_agent(),
    }))
}

fn client() -> TestClient {
    let router: OxideRouter<()> = routes! {
        prefix("/api") {
            post("/echo", echo);
            get("/posts/{post}", who).name("posts.show");
            get("/whoami", who);
        }
    };
    TestClient::new(axum::Router::from(router).layer(DefaultBodyLimit::max(64)))
}

#[tokio::test]
async fn input_merges_the_query_string_and_a_json_body() {
    let response = client()
        .post("/api/echo?page=3&admin=yes&user[name]=FromQuery")
        .json(&json!({ "user": { "name": "Ann" }, "note": "  " }))
        .send()
        .await;
    response.assert_ok();
    assert_eq!(response.json()["name"], "Ann", "the body wins");
    assert_eq!(response.json()["page"], 3);
    assert_eq!(response.json()["admin"], true);
    assert_eq!(response.json()["filled_note"], false);
    assert_eq!(response.json()["all"]["page"], "3");
}

#[tokio::test]
async fn input_reads_php_style_forms() {
    let response = client()
        .post("/api/echo")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("user[name]=Bo&tags[]=a&tags[]=b&admin=off")
        .send()
        .await;
    response.assert_ok();
    assert_eq!(response.json()["name"], "Bo");
    assert_eq!(response.json()["admin"], false);
    assert_eq!(response.json()["all"]["tags"], json!(["a", "b"]));
}

#[tokio::test]
async fn bad_bodies_are_answered_not_swallowed() {
    let client = client();
    let malformed = client
        .post("/api/echo")
        .header("Content-Type", "application/json")
        .body("{ nope")
        .send()
        .await;
    malformed.assert_bad_request();
    assert!(
        malformed.json()["message"]
            .as_str()
            .unwrap()
            .starts_with("Malformed JSON")
    );

    let big = client
        .post("/api/echo")
        .header("Content-Type", "application/json")
        .body(format!(r#"{{"x":"{}"}}"#, "a".repeat(200)))
        .send()
        .await;
    assert_eq!(big.status().as_u16(), 413);
    assert_eq!(big.json(), json!({ "message": "Payload Too Large" }));
}

#[tokio::test]
async fn request_ext_answers_questions_about_the_request() {
    let client = client();
    let api = client
        .get("/api/posts/7")
        .header("Accept", "application/json")
        .header("Authorization", "Bearer tok-1")
        .header("User-Agent", "tests/1")
        .header("Host", "app.test")
        .send()
        .await
        .json();
    assert_eq!(api["wants_json"], true);
    assert_eq!(api["bearer"], "tok-1");
    assert_eq!(api["route"], "posts.show");
    assert_eq!(api["posts_route"], true);
    assert_eq!(api["api_path"], true);
    assert_eq!(api["agent"], "tests/1");
    assert_eq!(api["url"], "http://app.test/api/posts/7");

    let browser = client
        .get("/api/whoami")
        .header("Accept", "text/html,*/*;q=0.8")
        .send()
        .await
        .json();
    assert_eq!(browser["wants_json"], false);
    assert_eq!(
        browser["route"],
        Value::Null,
        "an unnamed route has no name"
    );
    assert_eq!(browser["posts_route"], false);
    assert_eq!(browser["bearer"], Value::Null);
}
