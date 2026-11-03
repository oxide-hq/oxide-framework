//! Scoped bindings and custom binding keys.
#![allow(clippy::unused_async_trait_impl)]

use std::str::FromStr;

use oxide_http::prelude::*;

#[derive(Clone)]
struct User(u64);

#[derive(Clone)]
struct Post {
    slug: String,
    owner: u64,
}

#[derive(Clone)]
struct Db;

impl RouteBindable<Db> for User {
    type Id = u64;

    fn param_name() -> &'static str {
        "user"
    }

    async fn bind_from_state(id: u64, _: &Db) -> Result<Option<Self>, OxideHttpError> {
        Ok((id < 10).then_some(Self(id)))
    }
}

/// Posts are looked up by slug, not by numeric id.
impl ScopedBindable<User, Db> for Post {
    type Id = Slug;

    fn param_name() -> &'static str {
        "post"
    }

    async fn bind_in(user: &User, slug: Slug, _: &Db) -> Result<Option<Self>, OxideHttpError> {
        let all = [("hello", 1), ("other", 2)];
        Ok(all
            .iter()
            .find(|(candidate, owner)| *candidate == slug.0 && *owner == user.0)
            .map(|(candidate, owner)| Self {
                slug: (*candidate).to_owned(),
                owner: *owner,
            }))
    }
}

struct Slug(String);

impl FromStr for Slug {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self(value.to_owned()))
    }
}

async fn show(BindIn(user, post): BindIn<User, Post>) -> String {
    format!("{}:{}:{}", user.0, post.slug, post.owner)
}

#[tokio::test]
async fn children_are_found_only_inside_their_parent() {
    let router: OxideRouter<Db> = routes! {
        get("/users/{user}/posts/{post}", show);
    };
    let client = TestClient::new(router.with_state(Db));
    let ok = client.get("/users/1/posts/hello").send().await;
    assert_eq!(ok.text(), "1:hello:1");
    assert_eq!(
        client
            .get("/users/2/posts/hello")
            .send()
            .await
            .status()
            .as_u16(),
        404
    );
    assert_eq!(
        client
            .get("/users/99/posts/hello")
            .send()
            .await
            .status()
            .as_u16(),
        404
    );
}
