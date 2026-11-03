//! Fallback and redirect routes.

use axum::http::StatusCode;
use oxide_http::prelude::*;

async fn home() -> &'static str {
    "home"
}

async fn missing() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "custom 404")
}

#[tokio::test]
async fn fallback_and_redirect_routes() {
    let router: OxideRouter<()> = routes! {
        get("/", home);
        get("/old", redirect_to("/"));
        get("/moved", permanent_redirect_to("/"));
    };
    let client = TestClient::new(axum::Router::from(router.fallback(missing)));
    let old = client.get("/old").send().await;
    assert_eq!(old.status(), StatusCode::FOUND);
    assert_eq!(old.headers()["location"], "/");
    assert_eq!(
        client.get("/moved").send().await.status(),
        StatusCode::MOVED_PERMANENTLY
    );
    let nope = client.get("/nope").send().await;
    assert_eq!(nope.status(), StatusCode::NOT_FOUND);
    assert_eq!(nope.text(), "custom 404");
}
