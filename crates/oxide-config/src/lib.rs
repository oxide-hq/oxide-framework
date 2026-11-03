//! # oxide-config
//!
//! Typed configuration, Laravel-style: one struct per `config/*.toml` file, overridable from the
//! environment and `.env` files, checked once at boot instead of failing at first use.
//!
//! ```ignore
//! #[derive(Clone, Deserialize, Config)]
//! #[config(file = "app")]
//! struct AppConfig {
//!     name: String,
//!     #[config(env = "APP_PORT", default = 3000)]
//!     port: u16,
//!     #[config(env = "APP_KEY", secret)]
//!     key: String,
//! }
//!
//! let config = Configuration::builder().section::<AppConfig>().load()?;
//! let app = config.get::<AppConfig>().unwrap();
//! ```
//!
//! Put [`Configuration`] in your service container and handlers can take `Config<AppConfig>`.

/// `#[derive(Config)]` takes one concrete struct: generics and `rename_all` are compile errors.
///
/// ```compile_fail
/// #[derive(serde::Deserialize, oxide_config::Config)]
/// #[config(file = "x")]
/// struct Section<T> {
///     value: T,
/// }
/// ```
///
/// ```compile_fail
/// #[derive(serde::Deserialize, oxide_config::Config)]
/// #[config(file = "x")]
/// #[serde(rename_all = "camelCase")]
/// struct Section {
///     max_size: u8,
/// }
/// ```
#[cfg(doctest)]
pub struct DeriveLimits;

// Lets the derive macro's `::oxide_config::...` paths resolve inside this crate too.
extern crate self as oxide_config;

mod environment;
mod error;
mod extract;
mod loader;
mod value;

pub use environment::Environment;
pub use error::{ConfigErrors, ConfigIssue};
pub use extract::{Config, config_of};
pub use loader::{ConfigBuilder, ConfigSection, Configuration, DefaultFn, EnvCoerce, FieldSpec};
pub use oxide_config_macros::Config;
pub use value::EnvValue;

/// Re-exports used by generated code. Not public API.
#[doc(hidden)]
pub mod __private {
    pub use crate::loader::{to_value, validate};
    pub use toml::Value as TomlValue;
}

/// `use oxide_config::prelude::*;`
pub mod prelude {
    pub use crate::{Config, ConfigErrors, ConfigSection, Configuration, Environment};
}
