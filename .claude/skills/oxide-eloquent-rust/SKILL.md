---
name: oxide-eloquent-rust
description: How to write code in the Oxide framework repository (Laravel ergonomics, zero-cost Rust). Use before adding or changing any handler, controller, FormRequest, route, Model, query, service or test in this workspace, and when reviewing such a change. Covers the three-layer design trace, early-return style, the five-action controller rule, the hard safety rules (no unwrap/expect, no MutexGuard across .await, bound SQL only) and the Oxide APIs to reach for.
---

# Oxide Eloquent Rust

Oxide gives Rust developers Laravel's expressiveness — a service container, `FormRequest`s,
route-model binding, resource routes, Eloquent models, a fluent test client — with every check
done by the compiler and no runtime reflection. Code in this repository should read like a Laravel
app and compile like hand-written Axum.

## 1. Think in three layers before writing code

For every feature, write down (in the plan or PR description) one line per layer:

1. **Domain intent** — what the user of the app is trying to do, in domain words.
   *"An editor publishes a draft post."*
2. **Laravel ergonomic mapping** — how a Laravel developer would express it.
   *"`POST /posts/{post}/publish` → invokable `PublishPost` controller, `PublishPostRequest`
   authorizes editors, route-model binding loads the post, `$post->save()`."*
3. **Zero-cost Rust mechanics** — the Oxide/Rust types that deliver it with no runtime cost.
   *"`routes!` entry named `posts.publish`; `Validated<PublishPostRequest>` with
   `#[form_request(authorize)]`; `Bind<Post>` from `#[derive(Model)]`; `post.save(&db).await?`;
   errors flow through `?` into `OxideHttpError`."*

If layer 3 needs `Box<dyn ...>`, `Any`, string-typed lookups or a runtime registry where a trait
bound or a generated method would do, rethink layer 2 first.

## 2. Early returns, happy path last

Spatie's Laravel guideline — handle failure first, keep the success path unindented at the end —
maps directly onto `let ... else` and `?`.

```rust
// Do
async fn show(db: DatabasePool, Path(slug): Path<String>) -> Result<Json<Post>, OxideHttpError> {
    let Some(post) = Post::query().where_slug(slug).first(&db).await? else {
        return Err(OxideHttpError::NotFound("No query results for model [post]".into()));
    };
    if !post.published {
        return Err(OxideHttpError::Forbidden(String::new()));
    }
    Ok(Json(post))
}

// Don't
async fn show(/* ... */) -> Result<Json<Post>, OxideHttpError> {
    match Post::query().where_slug(slug).first(&db).await {
        Ok(Some(post)) => {
            if post.published { Ok(Json(post)) } else { Err(/* ... */) }
        }
        Ok(None) => Err(/* ... */),
        Err(e) => Err(e.into()),
    }
}
```

- Propagate with `?`; convert with `From` impls (`OrmError` → `OxideHttpError` already exists).
- No `if`/`else` pyramids and no `match` used only to unwrap-or-return; reach for `let ... else`,
  `?`, `ok_or_else`, `map_err`.
- Prefer `first_or_fail` / `find_or_fail` / `Bind<T>` over fetching an `Option` and 404-ing by hand.

## 3. Controllers have five actions

A resource controller may define only `index`, `store`, `show`, `update` and `destroy`, which
`resource("/posts", PostController)` wires up and names `posts.index` … `posts.destroy`.

Any other verb becomes its own single-action ("invokable") controller with one `handle` method,
or an Action struct the handler calls:

```rust
/// `POST /posts/{post}/publish`
struct PublishPost;

impl PublishPost {
    async fn handle(db: DatabasePool, Bind(mut post): Bind<Post>) -> Result<Json<Post>, OrmError> {
        post.published = true;
        post.save(&db).await?;
        Ok(Json(post))
    }
}

let router: OxideRouter<AppContext> = routes! {
    prefix("/api/v1") {
        resource("/posts", PostController);
        post("/posts/{post}/publish", PublishPost::handle).name("posts.publish");
    }
};
```

Keep handlers thin: extract (`Validated`, `Bind`, services), call the model or an Action, return.
Business logic that is reused or has more than a couple of steps goes in an Action struct.

## 4. Hard rules

These are review blockers.

- **No `.unwrap()` / `.expect()` / `panic!` in library or handler code.** Return a `Result` and
  use `?`. Allowed only in tests, in test-support helpers whose job is to fail the test
  (`TestClient`), in proc-macro code paths the parser has already ruled out (prefer
  `syn::Error` anyway), and where the API is documented to panic (`route!`, with a `# Panics`
  section).
- **No `std::sync::MutexGuard` (or `RwLock` guard) held across `.await`.** Lock, copy or mutate,
  drop — then await. Scope the guard in a block or use a short helper method. If state truly must
  stay locked across an await, use `tokio::sync::Mutex`.
- **Every SQL value is a bound `?` parameter.** Never `format!` a value into SQL. Use the
  generated `where_*` / `order_by_*` methods, or the public `Query` builder (`filter`,
  `filter_in`, `order_by`), which bind values and quote identifiers internally; the
  `_contains` / `_starts_with` / `_ends_with` methods escape `LIKE` wildcards for you.
  Hand-written SQL through `sqlx` must bind values with `.bind(..)`.
- **No `unsafe`.** The workspace forbids it.
- **Clippy pedantic stays clean** with `-D warnings`; allow a lint only locally, with a comment
  saying why.

## 5. Reach for the Oxide API first

| Need | Use |
| ---- | --- |
| Settings | `#[derive(Deserialize, Config)] #[config(file = "app")]` struct; `#[config(env = "APP_PORT", default = 3000, secret)]` per field; load with `Configuration::builder().section::<AppConfig>().load()`; read via the `Config<AppConfig>` extractor or `config_of::<Ctx, AppConfig>()`. Don't derive `Debug` on a section (the derive writes one that hides secrets) |
| Collection-style helpers | `CollectionExt` on any iterator (`group_by`, `key_by`, `sum_by`, `sorted_by_desc`, lazy `chunk`/`unique_by`); `Tap` (`pipe`/`tap`/`when`); `Str`/`Str::of(..)`, `Arr` (dot paths on JSON), `Number`, `diff_for_humans`, `Pipeline`, `Lottery`, `retry`. Prefer `Iterator` methods where `std` already has the equivalent (`map`, `filter`, `partition`, `find`, `fold`) |
| Inject a service into a handler | `#[derive(Clone, Service)]` on the service, `#[derive(Clone, ServiceContainer)]` on the app state, then take `mailer: Mailer` as a handler argument |
| Facade-style access | `AppContext::resolve::<Mailer>()` inside `facade_middleware` |
| Validate and authorize input | `#[derive(Deserialize, Validate, FormRequest)]` (+ `#[form_request(authorize)]` and `impl Authorize<S>`), extract `Validated<T>` |
| Load a model from `{post}` | `Bind<Post>` — automatic for `#[derive(Model)]`; `impl RouteBindable<S>` by hand otherwise |
| Routes | `routes! { prefix(..) { resource(..); get(..).name(..); middleware(layer) { .. } } }` |
| URLs | `route!("posts.show", post = 1)`, `url_for`, or the `UrlGenerator` extractor in handlers |
| Database | `DatabasePool` (a `Service`; `connect(url)` picks SQLite or PostgreSQL from the scheme, `driver()` says which); `#[derive(sqlx::FromRow, Model)]` |
| Schema | `db.migrate(&sqlx::migrate!("./database/migrations"))`; tests use `DatabasePool::connect_fresh(url)` for an isolated database |
| Queries | `Post::query().where_published(true).where_title_contains("x").order_by_id_desc().limit(10).get(&db)` |
| Writes | `Post::insert(&db, &NewPost { .. })`, `post.save(&db)`, `post.delete(&db)` |
| Errors | `OxideHttpError` (400/403/404/422/500, Laravel-shaped JSON); `OrmError` converts into it |
| Tests | `TestClient::new(router.with_state(ctx))` with chained `assert_*` |
| New model, controller or request in an app | `oxide make:model Comment -m`, `oxide make:controller Comment --resource`, `oxide make:request StoreCommentRequest`: they write the file and register its module (then add the route yourself) |

Model notes: the primary key is the field named `id` or `#[model(primary_key)]`; the table
defaults to the snake_case plural; column names follow `#[sqlx(rename)]` / `#[sqlx(rename_all)]`.
With the `postgres` feature, `i64` fields need `BIGINT`/`BIGSERIAL` columns, and LIKE-based
`_contains` is case-sensitive. Apps depend on the `oxide` umbrella crate and import
`oxide::prelude::*`.

## 6. Tests

- Every behaviour gets an integration test under `crates/<crate>/tests/` that goes through HTTP
  with `TestClient` where a route is involved: status, JSON paths
  (`assert_json_path("data.0.title", ..)`), validation errors
  (`assert_validation_error_for("title")`), and the database state afterwards.
- Use `DatabasePool::connect_fresh(url)` plus `migrate` (or `connect_in_memory()` plus `execute_schema`) for ORM tests. ORM cases go in `crates/oxide-orm/tests/suite`, which runs against SQLite and PostgreSQL.
- Cover the failure paths (404, 403, 422, 500) as well as the happy path.
- Fixing a bug starts with a test that fails without the fix.

## 7. Repository conventions

- Rust 2024, MSRV **1.88**: don't use newer std APIs (CI runs `cargo +1.88 check`). This is why
  sqlx stays on 0.8.
- Dependencies are declared once in the root `[workspace.dependencies]`.
- Generated code refers to dependencies through `::oxide_http::__private` /
  `::oxide_orm::__private`, so user crates don't need direct `axum` or `tokio` dependencies.
- Pedantic lints that come up often:
  - `async fn` in a trait impl with no `.await` → return `std::future::ready(..)`;
  - `assert!(x.is_empty())` → `assert_eq!(x.len(), 0)`;
  - `.map(format!(..)).collect::<String>()` → a `write!` loop.
- Before pushing, run `just ci` (fmt-check, clippy `-D warnings`, tests), `just coverage-check`
  (workspace line coverage must stay ≥ 85%), plus
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.

## Review checklist

- [ ] The PR states the three layers for each new feature.
- [ ] Failure is handled first; no nested `if`/`else` or unwrap-or-return `match`.
- [ ] Controllers expose only the five resource actions; extra verbs are invokable controllers
      or Actions.
- [ ] No `unwrap`/`expect`/`panic!` beyond the exceptions in section 4; no lock guard across
      `.await`.
- [ ] All SQL values are bound; no `format!` into SQL.
- [ ] Integration tests cover success and failure paths through HTTP.
- [ ] `just ci` and the rustdoc check pass; MSRV 1.88 still compiles.
