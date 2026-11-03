//! `api_resource` and the `names` override.
#![allow(dead_code, clippy::unused_async, clippy::unused_async_trait_impl)]

use oxide_http::prelude::*;

struct Photos;

impl Photos {
    async fn index() -> &'static str {
        "index"
    }
    async fn store() -> &'static str {
        "store"
    }
    async fn show() -> &'static str {
        "show"
    }
    async fn update() -> &'static str {
        "update"
    }
    async fn destroy() -> &'static str {
        "destroy"
    }
}

#[test]
fn names_override_the_route_name_prefix() {
    let router: OxideRouter<()> = routes! {
        api_resource("/photos", Photos, names = "gallery", only = [index, show]);
        resource("/pictures/{picture}/tags", Photos, param = "tag", names = "pic.tags", only = [show]);
    };
    let registry = router.registry();
    assert!(registry.get("gallery.index").is_some());
    assert!(registry.get("gallery.show").is_some());
    assert!(registry.get("gallery.store").is_none());
    assert_eq!(
        registry
            .url("pic.tags.show", &[("picture", "1"), ("tag", "x")])
            .unwrap(),
        "/pictures/1/tags/x"
    );
}
