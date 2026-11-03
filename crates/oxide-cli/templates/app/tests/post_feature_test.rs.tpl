//! Feature tests for the `/api/v1/posts` resource. Each test gets its own fresh database.

use oxide::prelude::*;
use serde_json::json;
use {{crate_name}}::models::Post;
use {{crate_name}}::{AppContext, MIGRATOR, build_app};

{{database_url_fn}}

async fn app() -> (AppContext, TestClient) {
    let db = DatabasePool::connect_fresh(&database_url()).await.unwrap();
    db.migrate(&MIGRATOR).await.unwrap();
    let ctx = AppContext::new(db, {{crate_name}}::config::load().unwrap());
    (ctx.clone(), TestClient::new(build_app(ctx)))
}

#[tokio::test]
async fn it_lists_the_seeded_posts() {
    let (_, client) = app().await;
    client
        .get("/api/v1/posts")
        .send()
        .await
        .assert_ok()
        .assert_json_path("0.title", "Welcome to Oxide")
        .assert_json_path("1.published", false);
}

#[tokio::test]
async fn it_shows_a_post_or_answers_404() {
    let (_, client) = app().await;
    client
        .get("/api/v1/posts/1")
        .send()
        .await
        .assert_ok()
        .assert_json_path("title", "Welcome to Oxide");
    client.get("/api/v1/posts/999").send().await.assert_not_found();
}

#[tokio::test]
async fn it_stores_a_valid_post_and_sends_mail() {
    let (ctx, client) = app().await;
    client
        .post("/api/v1/posts")
        .json(&json!({ "title": "Hello Oxide", "body": "Written in Rust.", "published": true }))
        .send()
        .await
        .assert_created()
        .assert_json_path("id", 3)
        .assert_json_path("title", "Hello Oxide");
    assert!(ctx.mailer.was_sent("post_created"));
    assert_eq!(Post::query().count(&ctx.db).await.unwrap(), 3);
}

#[tokio::test]
async fn it_rejects_an_invalid_post_with_422() {
    let (ctx, client) = app().await;
    client
        .post("/api/v1/posts")
        .json(&json!({ "title": "Hi", "body": "" }))
        .send()
        .await
        .assert_unprocessable()
        .assert_validation_error_for("title")
        .assert_validation_error_for("body");
    assert!(!ctx.mailer.was_sent("post_created"));
}

#[tokio::test]
async fn it_updates_a_post() {
    let (ctx, client) = app().await;
    client
        .put("/api/v1/posts/2")
        .json(&json!({ "title": "Published at last", "body": "No longer a draft.", "published": true }))
        .send()
        .await
        .assert_ok()
        .assert_json_path("published", true);
    assert!(ctx.mailer.was_sent("post_updated"));
    assert_eq!(Post::find_or_fail(&ctx.db, 2).await.unwrap().title, "Published at last");
}

#[tokio::test]
async fn it_deletes_a_post() {
    let (_, client) = app().await;
    client.delete("/api/v1/posts/1").send().await.assert_no_content();
    client.get("/api/v1/posts/1").send().await.assert_not_found();
}
