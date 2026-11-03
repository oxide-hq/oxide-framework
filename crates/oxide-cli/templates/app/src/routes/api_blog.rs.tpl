//! The JSON API, under `/api/v1`.

use oxide::prelude::*;

use crate::context::AppContext;
use crate::http::controllers::post_controller::PostController;

/// `GET /api/v1/health`
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

pub fn router() -> OxideRouter<AppContext> {
    routes! {
        prefix("/api/v1") {
            get("/health", health).name("health");
            resource("/posts", PostController);
        }
    }
}
