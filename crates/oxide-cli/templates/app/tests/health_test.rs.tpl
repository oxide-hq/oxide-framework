//! The health check answers without touching any data.

use oxide::prelude::*;
use {{crate_name}}::{AppContext, build_app};
{{database_url_fn}}

#[tokio::test]
async fn the_health_check_is_ok() {
    let ctx = {{health_context}};
    let response = TestClient::new(build_app(ctx).await.unwrap())
        .get("/api/v1/health")
        .send()
        .await;
    response.assert_ok().assert_json_path("status", "ok");
    // `build_app` gives every request an id and returns it.
    assert!(response.headers().contains_key("x-request-id"));
}
