//! The health check answers without touching any data.

use oxide::prelude::*;
use {{crate_name}}::{AppContext, build_app};
{{database_url_fn}}

#[tokio::test]
async fn the_health_check_is_ok() {
    let ctx = {{health_context}};
    TestClient::new(build_app(ctx))
        .get("/api/v1/health")
        .send()
        .await
        .assert_ok()
        .assert_json_path("status", "ok");
}
