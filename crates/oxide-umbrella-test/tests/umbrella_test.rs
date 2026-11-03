//! An app that depends only on `oxide`: every derive and `routes!` must resolve through the
//! umbrella crate's `::oxide::http` / `::oxide::orm` paths.

// Axum handlers are async by contract even when they don't await.
#![allow(clippy::unused_async, clippy::unused_async_trait_impl)]

use oxide::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, sqlx::FromRow, Model)]
#[model(table = "podcasts")]
pub struct Podcast {
    pub id: i64,
    pub title: String,
}

#[derive(Clone, Debug, Service)]
pub struct Greeter {
    pub prefix: String,
}

impl Greeter {
    fn greet(&self, title: &str) -> String {
        format!("{}: {title}", self.prefix)
    }
}

#[derive(Clone, Debug, ServiceContainer)]
pub struct AppContext {
    pub db: DatabasePool,
    pub greeter: Greeter,
}

#[derive(Debug, Deserialize, Validate, FormRequest)]
pub struct StorePodcastRequest {
    #[validate(length(min = 3, message = "The title must be at least 3 characters."))]
    pub title: String,
}

struct PodcastController;

impl PodcastController {
    async fn index(db: DatabasePool) -> Result<Json<Vec<Podcast>>, OrmError> {
        Ok(Json(Podcast::query().order_by_id_asc().get(&db).await?))
    }

    async fn store(
        db: DatabasePool,
        Validated(req): Validated<StorePodcastRequest>,
    ) -> Result<(StatusCode, Json<Podcast>), OrmError> {
        let podcast = Podcast::create(&db, NewPodcast { title: req.title }).await?;
        Ok((StatusCode::CREATED, Json(podcast)))
    }

    async fn show(greeter: Greeter, Bind(podcast): Bind<Podcast>) -> Json<serde_json::Value> {
        Json(json!({ "id": podcast.id, "greeting": greeter.greet(&podcast.title) }))
    }

    async fn update(Bind(podcast): Bind<Podcast>) -> Json<Podcast> {
        Json(podcast)
    }

    async fn destroy(db: DatabasePool, Bind(podcast): Bind<Podcast>) -> OxideResult<StatusCode> {
        podcast.delete(&db).await?;
        Ok(StatusCode::NO_CONTENT)
    }
}

#[tokio::test]
async fn an_app_built_from_the_prelude_alone() {
    let db = DatabasePool::connect_fresh("sqlite::memory:")
        .await
        .unwrap();
    db.execute_schema("CREATE TABLE podcasts (id INTEGER PRIMARY KEY, title TEXT NOT NULL)")
        .await
        .unwrap();

    let router: OxideRouter<AppContext> = routes! {
        prefix("/api") {
            resource("/podcasts", PodcastController);
        }
    };
    let client = TestClient::new(router.with_state(AppContext {
        db,
        greeter: Greeter {
            prefix: "Now playing".into(),
        },
    }));

    client
        .post("/api/podcasts")
        .json(&json!({ "title": "ab" }))
        .send()
        .await
        .assert_validation_error_for("title");
    client
        .post("/api/podcasts")
        .json(&json!({ "title": "Oxide FM" }))
        .send()
        .await
        .assert_created()
        .assert_json_path("id", 1);
    client
        .get("/api/podcasts/1")
        .send()
        .await
        .assert_ok()
        .assert_json_path("greeting", "Now playing: Oxide FM");
    client.get("/api/podcasts").send().await.assert_ok();
    assert_eq!(
        oxide::url_for("podcasts.show", &[("podcast", "1")]).unwrap(),
        "/api/podcasts/1"
    );
    client
        .delete("/api/podcasts/1")
        .send()
        .await
        .assert_no_content();
    client
        .get("/api/podcasts/1")
        .send()
        .await
        .assert_not_found();
}
