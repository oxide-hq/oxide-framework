//! Route constraints through real routes.

use oxide_http::prelude::*;

async fn ok() -> &'static str {
    "ok"
}

fn client() -> TestClient {
    let router: OxideRouter<()> = routes! {
        middleware(where_number("post")) {
            get("/posts/{post}", ok);
        }
        middleware(where_in("status", &["draft", "published"])) {
            get("/by/{status}", ok);
        }
        middleware(where_uuid("id")) {
            get("/things/{id}", ok);
        }
        middleware(where_alpha("slug")) {
            get("/tags/{slug}", ok);
        }
        middleware(where_alpha_numeric("code")) {
            get("/codes/{code}", ok);
        }
        middleware(where_fn("x", |v| v.len() == 2)) {
            get("/two/{x}", ok);
        }
    };
    TestClient::new(axum::Router::from(router))
}

#[tokio::test]
async fn invalid_parameters_are_not_found() {
    let client = client();
    for (path, status) in [
        ("/posts/7", 200),
        ("/posts/seven", 404),
        ("/by/draft", 200),
        ("/by/other", 404),
        ("/things/123e4567-e89b-12d3-a456-426614174000", 200),
        ("/things/nope", 404),
        ("/tags/rust", 200),
        ("/tags/rust2", 404),
        ("/codes/ab12", 200),
        ("/codes/a-b", 404),
        ("/two/ab", 200),
        ("/two/abc", 404),
    ] {
        let response = client.get(path).send().await;
        assert_eq!(response.status().as_u16(), status, "{path}");
    }
}
