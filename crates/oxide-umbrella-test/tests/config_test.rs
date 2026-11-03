//! `#[derive(Config)]` resolves through `::oxide::config` when only `oxide` is a dependency.

use oxide::prelude::*;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Clone, Deserialize, Config)]
#[config(file = "site")]
struct SiteConfig {
    #[config(env = "SITE_NAME", default = "Oxide")]
    name: String,
    #[config(env = "SITE_TOKEN", secret, default = "t")]
    token: String,
}

#[test]
fn derive_config_works_through_the_umbrella() {
    let configuration = Configuration::builder()
        .vars([("SITE_NAME", "Umbrella")])
        .section::<SiteConfig>()
        .load()
        .unwrap();
    let site = configuration.get::<SiteConfig>().unwrap();
    assert_eq!(site.name, "Umbrella");
    assert!(format!("{site:?}").contains("[REDACTED]"));
    assert_eq!(configuration.environment(), &Environment::Local);
}
