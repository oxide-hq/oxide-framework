//! Typed configuration: each struct is read from `config/<file>.toml`, then overridden by the
//! environment variable named in `env` (process environment first, then `.env`).

use oxide::prelude::*;
use serde::Deserialize;

/// `config/app.toml`
#[derive(Clone, Deserialize, Config)]
#[config(file = "app")]
pub struct AppConfig {
    #[config(env = "APP_NAME")]
    pub name: String,
    #[config(env = "APP_PORT", default = 3000)]
    pub port: u16,
    #[config(env = "APP_DEBUG", default = false)]
    pub debug: bool,
}
{{database_config}}
/// Loads and validates every section at once; the error lists everything that is wrong.
pub fn load() -> Result<Configuration, ConfigErrors> {
    Configuration::builder()
        .section::<AppConfig>()
        .section::<LoggingConfig>(){{database_section}}
        .load()
}
