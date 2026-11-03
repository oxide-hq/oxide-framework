//! The application bootstrap is reachable from `oxide::prelude` alone.

use std::sync::Arc;

use oxide::prelude::*;

trait Greeter: Send + Sync {
    fn greet(&self) -> &'static str;
}

struct English;

impl Greeter for English {
    fn greet(&self) -> &'static str {
        "hello"
    }
}

struct GreeterProvider;

impl ServiceProvider for GreeterProvider {
    fn register(&self, bindings: &mut Bindings) {
        bindings.singleton_arc::<dyn Greeter>(Arc::new(English));
    }
}

async fn greet(greeter: Inject<dyn Greeter>) -> &'static str {
    greeter.greet()
}

#[tokio::test]
async fn an_application_boots_from_the_prelude() {
    let app = Application::builder()
        .provider(GreeterProvider)
        .routes(oxide::axum::Router::new().route("/", oxide::axum::routing::get(greet)))
        .build()
        .await
        .unwrap();
    let response = app.test_client().get("/").send().await;
    response.assert_ok();
    assert_eq!(response.text(), "hello");
}
