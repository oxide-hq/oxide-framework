use oxide::prelude::*;

use crate::context::AppMailer;
use crate::http::requests::store_post_request::StorePostRequest;
use crate::models::post::{NewPost, Post};

/// The `/posts` resource: `resource("/posts", PostController)` routes the five actions.
#[derive(Debug)]
pub struct PostController;

impl PostController {
    /// `GET /posts`
    pub async fn index(db: DatabasePool) -> OxideResult<Json<Vec<Post>>> {
        Ok(Json(Post::query().order_by_id_asc().get(&db).await?))
    }

    /// `POST /posts`
    pub async fn store(
        db: DatabasePool,
        mailer: AppMailer,
        Validated(request): Validated<StorePostRequest>,
    ) -> OxideResult<(StatusCode, Json<Post>)> {
        let post = Post::create(
            &db,
            NewPost {
                title: request.title,
                body: request.body,
                published: request.published,
            },
        )
        .await?;
        mailer.send("post_created");
        Ok((StatusCode::CREATED, Json(post)))
    }

    /// `GET /posts/{post}`
    pub async fn show(Bind(post): Bind<Post>) -> Json<Post> {
        Json(post)
    }

    /// `PUT /posts/{post}`
    pub async fn update(
        db: DatabasePool,
        mailer: AppMailer,
        Bind(mut post): Bind<Post>,
        Validated(request): Validated<StorePostRequest>,
    ) -> OxideResult<Json<Post>> {
        post.title = request.title;
        post.body = request.body;
        post.published = request.published;
        post.save(&db).await?;
        mailer.send("post_updated");
        Ok(Json(post))
    }

    /// `DELETE /posts/{post}`
    pub async fn destroy(db: DatabasePool, Bind(post): Bind<Post>) -> OxideResult<StatusCode> {
        post.delete(&db).await?;
        Ok(StatusCode::NO_CONTENT)
    }
}
