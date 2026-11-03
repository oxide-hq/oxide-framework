//! Phase 3: Eloquent-style queries plus the full HTTP + SQLite lifecycle through `Bind<Post>`.

// Axum handlers are async by contract even when they don't await.
#![allow(clippy::unused_async, clippy::unused_async_trait_impl)]

use axum::Json;
use axum::http::StatusCode;
use chrono::NaiveDate;
use oxide_http::prelude::*;
use oxide_orm::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

const SCHEMA: &str = r"
    CREATE TABLE posts (
        id        INTEGER PRIMARY KEY AUTOINCREMENT,
        author_id INTEGER NOT NULL,
        title     TEXT    NOT NULL,
        body      TEXT    NOT NULL,
        published BOOLEAN NOT NULL DEFAULT 0
    );
    CREATE TABLE comments (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        post_id    INTEGER NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
        body       TEXT    NOT NULL,
        rating     INTEGER,
        created_on DATE    NOT NULL
    );
";

// --- Models (no manual RouteBindable impls) ---------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow, Model)]
#[model(table = "posts")]
pub struct Post {
    #[model(primary_key)]
    pub id: i64,
    pub author_id: i64,
    pub title: String,
    pub body: String,
    pub published: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, Model)]
pub struct Comment {
    pub id: i64,
    pub post_id: i64,
    pub body: String,
    pub rating: Option<i64>,
    pub created_on: NaiveDate,
}

// --- HTTP layer -----------------------------------------------------------------------------

#[derive(Clone, Debug, ServiceContainer)]
struct AppContext {
    db: DatabasePool,
}

#[derive(Debug, Deserialize, Validate, FormRequest)]
struct CreatePostRequest {
    #[validate(range(min = 1, message = "The author id must be positive"))]
    author_id: i64,
    #[validate(length(min = 5, message = "Title must be at least 5 characters"))]
    title: String,
    #[validate(length(min = 1, message = "Body is required"))]
    body: String,
}

#[derive(Debug, Deserialize, Validate, FormRequest)]
struct UpdatePostRequest {
    #[validate(length(min = 5, message = "Title must be at least 5 characters"))]
    title: String,
    published: Option<bool>,
}

struct PostController;

impl PostController {
    async fn index(db: DatabasePool) -> Result<Json<Vec<Post>>, OrmError> {
        Ok(Json(Post::query().order_by_id_asc().get(&db).await?))
    }

    async fn store(
        db: DatabasePool,
        Validated(req): Validated<CreatePostRequest>,
    ) -> Result<(StatusCode, Json<Post>), OrmError> {
        let post = Post::insert(
            &db,
            &NewPost {
                author_id: req.author_id,
                title: req.title,
                body: req.body,
                published: false,
            },
        )
        .await?;
        Ok((StatusCode::CREATED, Json(post)))
    }

    async fn show(Bind(post): Bind<Post>) -> Json<Post> {
        Json(post)
    }

    async fn update(
        db: DatabasePool,
        Bind(mut post): Bind<Post>,
        Validated(req): Validated<UpdatePostRequest>,
    ) -> Result<Json<Post>, OrmError> {
        post.title = req.title;
        if let Some(published) = req.published {
            post.published = published;
        }
        post.save(&db).await?;
        Ok(Json(post))
    }

    async fn destroy(db: DatabasePool, Bind(post): Bind<Post>) -> Result<StatusCode, OrmError> {
        post.delete(&db).await?;
        Ok(StatusCode::NO_CONTENT)
    }
}

// --- Fixtures -------------------------------------------------------------------------------

async fn database() -> DatabasePool {
    let db = DatabasePool::connect_in_memory().await.unwrap();
    db.execute_schema(SCHEMA).await.unwrap();
    db
}

async fn seed(db: &DatabasePool) {
    let rows = [
        (3, "Hello Oxide", true),
        (10, "Oxide routing deep dive", true),
        (12, "Draft: Oxide ORM", false),
        (15, "Unrelated rust post", true),
        (20, "50% off_sale", false),
    ];
    for (author_id, title, published) in rows {
        Post::create(
            db,
            NewPost {
                author_id,
                title: title.into(),
                body: format!("Body of {title}"),
                published,
            },
        )
        .await
        .unwrap();
    }
}

async fn app() -> (DatabasePool, TestClient) {
    let db = database().await;
    let router: OxideRouter<AppContext> = routes! {
        prefix("/api") {
            resource("/posts", PostController);
        }
    };
    let client = TestClient::new(router.with_state(AppContext { db: db.clone() }));
    (db, client)
}

fn titles(posts: &[Post]) -> Vec<&str> {
    posts.iter().map(|p| p.title.as_str()).collect()
}

// --- Query builder --------------------------------------------------------------------------

#[tokio::test]
async fn fluent_queries_filter_and_order() {
    let db = database().await;
    seed(&db).await;

    let posts = Post::query()
        .where_published(true)
        .where_title_contains("Oxide")
        .order_by_id_desc()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(titles(&posts), ["Oxide routing deep dive", "Hello Oxide"]);

    assert_eq!(
        Post::query()
            .where_author_id_gte(10)
            .count(&db)
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        Post::query()
            .where_author_id_lt(10)
            .count(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        Post::query()
            .where_author_id_gt(10)
            .where_author_id_lte(15)
            .count(&db)
            .await
            .unwrap(),
        2
    );

    let drafts = Post::query()
        .where_published_not(true)
        .get(&db)
        .await
        .unwrap();
    assert_eq!(drafts.len(), 2);

    let picked = Post::query()
        .where_author_id_in(vec![3, 20])
        .order_by_author_id_asc()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(titles(&picked), ["Hello Oxide", "50% off_sale"]);
    assert_eq!(
        Post::query()
            .where_id_in(Vec::new())
            .count(&db)
            .await
            .unwrap(),
        0
    );

    assert_eq!(
        Post::query()
            .where_title_starts_with("Draft")
            .first(&db)
            .await
            .unwrap()
            .unwrap()
            .author_id,
        12
    );
    assert_eq!(
        Post::query()
            .where_title_ends_with("post")
            .count(&db)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn like_wildcards_in_user_input_are_escaped() {
    let db = database().await;
    seed(&db).await;

    // `%` and `_` are literal characters here, not wildcards.
    let sale = Post::query()
        .where_title_contains("50% off_")
        .get(&db)
        .await
        .unwrap();
    assert_eq!(titles(&sale), ["50% off_sale"]);
    assert_eq!(
        Post::query()
            .where_title_contains("%")
            .count(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        Post::query()
            .where_title_contains("_")
            .count(&db)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn pagination_and_terminal_methods() {
    let db = database().await;
    seed(&db).await;

    let page = Post::query()
        .order_by_id_asc()
        .limit(2)
        .offset(1)
        .get(&db)
        .await
        .unwrap();
    assert_eq!(page.iter().map(|p| p.id).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(Post::query().offset(3).count(&db).await.unwrap(), 2);
    assert_eq!(Post::all(&db).await.unwrap().len(), 5);

    let missing = Post::query()
        .where_author_id(999)
        .first_or_fail(&db)
        .await
        .unwrap_err();
    assert!(matches!(missing, OrmError::NotFound { model: "Post", .. }));

    let error = Post::find_or_fail(&db, 42).await.unwrap_err();
    assert_eq!(error.to_string(), "No query results for model [Post] 42");

    let removed = Post::query()
        .where_published(false)
        .delete(&db)
        .await
        .unwrap();
    assert_eq!(removed, 2);
    let removed = Post::query()
        .order_by_id_desc()
        .limit(1)
        .delete(&db)
        .await
        .unwrap();
    assert_eq!(removed, 1);
    assert_eq!(Post::all(&db).await.unwrap().len(), 2);
}

#[tokio::test]
async fn generated_sql_binds_every_value() {
    let (sql, values) = Post::query()
        .where_title_contains("x' OR 1=1 --")
        .where_published(true)
        .order_by_id_desc()
        .limit(5)
        .to_sql();
    assert_eq!(
        sql,
        r#"SELECT * FROM "posts" WHERE "title" LIKE ? ESCAPE '\' AND "published" = ? ORDER BY "id" DESC LIMIT ?"#
    );
    assert_eq!(values.len(), 3);
}

#[tokio::test]
async fn active_record_mutations_relations_and_options() {
    let db = database().await;
    seed(&db).await;

    let mut post = Post::find(&db, 1).await.unwrap().unwrap();
    post.title = "Hello Oxide, edited".into();
    post.save(&db).await.unwrap();
    assert_eq!(
        Post::find_or_fail(&db, 1).await.unwrap().title,
        "Hello Oxide, edited"
    );

    let day = NaiveDate::from_ymd_opt(2026, 11, 3).unwrap();
    for (body, rating) in [("Great", Some(5)), ("Meh", Some(2)), ("No rating", None)] {
        Comment::create(
            &db,
            NewComment {
                post_id: post.id,
                body: body.into(),
                rating,
                created_on: day,
            },
        )
        .await
        .unwrap();
    }

    let comments = post
        .has_many::<Comment>("post_id")
        .order_by_id_asc()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(comments.len(), 3);
    assert_eq!(
        post.has_many::<Comment>("post_id")
            .where_rating_gte(3)
            .count(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        Comment::query()
            .where_rating_null()
            .count(&db)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        Comment::query()
            .where_rating_not_null()
            .count(&db)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        Comment::query()
            .where_created_on(day)
            .count(&db)
            .await
            .unwrap(),
        3
    );

    let parent = comments[0]
        .belongs_to::<Post>("post_id")
        .first_or_fail(&db)
        .await
        .unwrap();
    assert_eq!(parent, post);

    post.delete(&db).await.unwrap();
    assert!(Post::find(&db, 1).await.unwrap().is_none());
    // ON DELETE CASCADE (foreign keys are enabled on every connection).
    assert_eq!(Comment::query().count(&db).await.unwrap(), 0);
    // Saving or deleting a row that no longer exists is a NotFound.
    assert!(matches!(
        post.save(&db).await,
        Err(OrmError::NotFound { .. })
    ));
    assert!(matches!(
        post.delete(&db).await,
        Err(OrmError::NotFound { .. })
    ));
}

// --- HTTP + database lifecycle -----------------------------------------------------------------

#[tokio::test]
async fn http_crud_lifecycle_through_bind() {
    let (db, client) = app().await;

    // store: validates, then inserts.
    client
        .post("/api/posts")
        .json(&json!({ "author_id": 0, "title": "abc", "body": "" }))
        .send()
        .await
        .assert_validation_error_for("author_id")
        .assert_validation_error_for("title")
        .assert_validation_error_for("body");
    assert_eq!(Post::query().count(&db).await.unwrap(), 0);

    client
        .post("/api/posts")
        .json(&json!({ "author_id": 7, "title": "Hello SQLite", "body": "Stored via Oxide" }))
        .send()
        .await
        .assert_created()
        .assert_json_path("id", 1)
        .assert_json_path("published", false);
    let stored = Post::find_or_fail(&db, 1).await.unwrap();
    assert_eq!(
        (stored.author_id, stored.title.as_str()),
        (7, "Hello SQLite")
    );

    // show: Bind<Post> queries SQLite by id.
    client
        .get("/api/posts/1")
        .send()
        .await
        .assert_ok()
        .assert_json_path("title", "Hello SQLite")
        .assert_json_path("body", "Stored via Oxide");
    client
        .get("/api/posts/999")
        .send()
        .await
        .assert_not_found()
        .assert_json_path("message", "No query results for model [post] 999");
    client.get("/api/posts/abc").send().await.assert_not_found();

    // index
    client
        .get("/api/posts")
        .send()
        .await
        .assert_ok()
        .assert_json_path("0.id", 1);

    // update: mutate + save, then verify the row.
    client
        .put("/api/posts/1")
        .json(&json!({ "title": "Renamed in SQLite", "published": true }))
        .send()
        .await
        .assert_ok()
        .assert_json_path("title", "Renamed in SQLite")
        .assert_json_path("published", true);
    let updated = Post::find_or_fail(&db, 1).await.unwrap();
    assert_eq!(updated.title, "Renamed in SQLite");
    assert!(updated.published);

    // destroy: delete, then the binding 404s.
    client
        .delete("/api/posts/1")
        .send()
        .await
        .assert_no_content();
    client.get("/api/posts/1").send().await.assert_not_found();
    assert_eq!(Post::query().count(&db).await.unwrap(), 0);
}

#[tokio::test]
async fn database_errors_render_as_opaque_500s() {
    let (db, client) = app().await;
    db.execute_schema("DROP TABLE comments; DROP TABLE posts;")
        .await
        .unwrap();

    client
        .get("/api/posts")
        .send()
        .await
        .assert_status(StatusCode::INTERNAL_SERVER_ERROR)
        .assert_json_path("message", "Server Error");
}
