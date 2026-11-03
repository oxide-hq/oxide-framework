<p align="center">
  <img src=".github/assets/banner.png" alt="Oxide — Laravel-grade ergonomics. Compile-time safety." width="100%">
</p>

<p align="center">
  <a href="https://github.com/oxide-hq/oxide-framework/actions/workflows/ci.yml"><img src="https://github.com/oxide-hq/oxide-framework/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg" alt="License: MIT OR Apache-2.0"></a>
  <a href="rust-toolchain.toml"><img src="https://img.shields.io/badge/MSRV-1.88-orange.svg" alt="MSRV 1.88"></a>
</p>

**Oxide** is a Laravel-inspired web framework for Rust. It aims to give you Laravel's developer
ergonomics — a service container, form requests, route-model binding, a fluent test client — while
everything is checked at compile time. It is built on [Axum 0.8](https://docs.rs/axum) and
[Tokio](https://tokio.rs).

> **Status:** early development (Phase 1). APIs will change.

## Features

| Laravel concept         | Oxide                                                                        |
| ----------------------- | ---------------------------------------------------------------------------- |
| Service container       | `#[derive(ServiceContainer)]` on your state, `#[derive(Service)]` on services |
| Facades                 | `AppContext::resolve::<Mailer>()` inside `facade_middleware` (task-local)     |
| `FormRequest`           | `#[derive(FormRequest)]` + the `Validated<T>` extractor                       |
| Route-model binding     | `impl RouteBindable` + the `Bind<T>` extractor                                |
| Error responses         | `OxideHttpError` → Laravel-shaped JSON for 400 / 403 / 404 / 422 / 500        |
| HTTP tests              | `TestClient` with chainable `assert_*` methods                                |

## Example

```rust
use std::sync::{Arc, Mutex};

use axum::http::request::Parts;
use axum::{Json, Router, routing::put};
use oxide_http::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Service)]
struct Mailer { sent: Arc<Mutex<Vec<String>>> }

#[derive(Clone, Debug, Default, Service)]
struct Db { /* connection pool, ... */ }

#[derive(Clone, Debug, Default, ServiceContainer)]
struct AppContext { mailer: Mailer, db: Db }

#[derive(Clone, Debug, Serialize)]
struct Post { id: u64, title: String }

impl RouteBindable<AppContext> for Post {
    type Id = u64;
    fn param_name() -> &'static str { "post" }
    async fn bind_from_state(id: u64, _ctx: &AppContext) -> Result<Option<Self>, OxideHttpError> {
        Ok((id == 1).then(|| Post { id, title: "Hello".into() }))
    }
}

#[derive(Debug, Deserialize, Validate, FormRequest)]
#[form_request(authorize)]
struct UpdatePost {
    #[validate(length(min = 5, message = "Title must be at least 5 characters"))]
    title: String,
}

impl Authorize<AppContext> for UpdatePost {
    async fn authorize(&self, parts: &Parts, _ctx: &AppContext) -> bool {
        parts.headers.get("x-user-role").is_some_and(|role| role == "editor")
    }
}

// Services, the bound model and the validated payload are all plain handler arguments.
async fn update_post(
    mailer: Mailer,
    Bind(mut post): Bind<Post>,
    Validated(req): Validated<UpdatePost>,
) -> Json<Post> {
    post.title = req.title;
    mailer.sent.lock().unwrap().push(format!("Post {} updated", post.id));
    Json(post)
}

fn app(ctx: AppContext) -> Router {
    Router::new()
        .route("/posts/{post}", put(update_post))
        .layer(axum::middleware::from_fn_with_state(ctx.clone(), facade_middleware::<AppContext>))
        .with_state(ctx)
}
```

`Validated<T>` accepts `application/json` and `application/x-www-form-urlencoded` bodies. It
returns `400` for a malformed body, `403` when `authorize` returns `false`, and `422` when
validation fails:

```json
{
  "message": "The given data was invalid.",
  "errors": { "title": ["Title must be at least 5 characters"] }
}
```

### Testing

```rust
#[tokio::test]
async fn editors_can_update_posts() {
    TestClient::new(app(AppContext::default()))
        .put("/posts/1")
        .header("X-User-Role", "editor")
        .json(&serde_json::json!({ "title": "A brand new title" }))
        .send()
        .await
        .assert_ok()
        .assert_json_path("title", "A brand new title");
}
```

`assert_json_path` accepts dot notation (`data.0.title`) or a JSON pointer (`/data/0/title`). Other
assertions include `assert_created`, `assert_not_found`, `assert_forbidden`,
`assert_unprocessable` and `assert_validation_error_for("field")`.

The full lifecycle suite is in
[`crates/oxide-http/tests/lifecycle_test.rs`](crates/oxide-http/tests/lifecycle_test.rs).

### Dependencies in your crate

The `Validate` derive comes from [`validator`](https://docs.rs/validator), and its generated code
refers to `::validator`. Your crate therefore needs `validator` (with the `derive` feature) and
`serde` as direct dependencies alongside `oxide-http`.

## Workspace

| Crate                                            | Purpose                                                          |
| ------------------------------------------------ | ---------------------------------------------------------------- |
| [`oxide-http`](crates/oxide-http)                | Extractors, error contract, service container traits, `TestClient` |
| [`oxide-http-macros`](crates/oxide-http-macros)  | `Service`, `ServiceContainer` and `FormRequest` derives (re-exported by `oxide-http`) |

## Development

You need stable Rust (pinned in [`rust-toolchain.toml`](rust-toolchain.toml)) and
[`just`](https://github.com/casey/just).
[`.cargo/config.toml`](.cargo/config.toml) links with `clang` + [`mold`](https://github.com/rui314/mold)
on x86_64 Linux; remove that block if you don't have them installed.

```bash
just test       # cargo test --workspace
just lint       # cargo clippy --workspace --all-targets -- -D warnings
just fmt        # cargo fmt --all
just ci         # fmt-check + lint + test
just deny       # cargo deny check (needs cargo-deny)
just expand     # cargo expand (needs cargo-expand)
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
