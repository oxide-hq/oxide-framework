//! # oxide-log
//!
//! Logging, Laravel-style, on [`tracing`]: channels configured in `config/logging.toml` (stderr,
//! a single file, daily files, JSON, and stacks of them), a request-scoped [`Context`] that
//! every log line carries, and middleware that gives each request an id.
//!
//! ```ignore
//! let config = Configuration::builder().section::<LoggingConfig>().load()?;
//! let _logs = Logger::init(&config.require::<LoggingConfig>()?)?;
//!
//! tracing::info!(user_id = 7, "signed in");     // structured, to the default channels
//! Log::channel("audit").info("exported report"); // to the `audit` channel only
//! ```

mod config;
mod context;
mod format;
mod log;
mod logger;
mod middleware;

pub use config::{ChannelConfig, Driver, LoggingConfig};
pub use context::Context;
pub use format::MemoryWriter;
pub use log::{ChannelLog, Log};
pub use logger::{LogError, LogGuard, Logger, LoggerBuilder};
pub use middleware::context_middleware;
pub use tracing;

/// `use oxide_log::prelude::*;`
pub mod prelude {
    pub use crate::{Context, Log, Logger, LoggingConfig};
}

// Lets the config derive's `::oxide_config::...` paths resolve inside this crate.
extern crate self as oxide_log;
