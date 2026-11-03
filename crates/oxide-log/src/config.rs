//! `config/logging.toml`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use oxide_config::Config;
use serde::Deserialize;

/// Where a channel writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Driver {
    /// Text lines on standard error.
    #[default]
    Stderr,
    /// Text lines appended to one file (`path`).
    Single,
    /// Text lines in a file per day (`path`, keeping `days` files).
    Daily,
    /// JSON lines on standard error, or in the file `path`.
    Json,
    /// Fans out to the channels in `channels`.
    Stack,
}

/// One channel under `[channels.<name>]`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelConfig {
    #[serde(default)]
    pub driver: Driver,
    /// The file for `single`, `daily` (`storage/logs/app.log` writes `app.<date>.log`) and
    /// `json`.
    pub path: Option<PathBuf>,
    /// The most verbose level this channel records (`error`, `warn`, `info`, `debug`, `trace`
    /// or `off`). Defaults to the top-level `level`.
    pub level: Option<String>,
    /// How many daily files to keep.
    pub days: Option<usize>,
    /// The members of a `stack`.
    #[serde(default)]
    pub channels: Vec<String>,
}

/// The logging section: which channel is the default and how each channel is set up.
///
/// ```toml
/// default = "stack"
///
/// [channels.stack]
/// driver = "stack"
/// channels = ["stderr", "daily"]
///
/// [channels.daily]
/// driver = "daily"
/// path = "storage/logs/app.log"
/// days = 14
///
/// [channels.audit]
/// driver = "json"
/// path = "storage/logs/audit.log"
/// ```
///
/// The default channel (and the channels in its stack) receives every event. Other channels,
/// like `audit` above, receive only events logged with `Log::channel("audit")`. A channel
/// called `stderr` always exists.
#[derive(Clone, Deserialize, Config)]
#[config(file = "logging")]
pub struct LoggingConfig {
    /// The channel ordinary events go to.
    #[config(env = "LOG_CHANNEL", default = "stderr")]
    pub default: String,
    /// The default level for channels that don't set their own.
    #[config(env = "LOG_LEVEL", default = "info")]
    pub level: String,
    #[serde(default)]
    pub channels: BTreeMap<String, ChannelConfig>,
}

impl LoggingConfig {
    /// A configuration that logs `level` and above to standard error.
    pub fn stderr(level: &str) -> Self {
        Self {
            default: "stderr".to_owned(),
            level: level.to_owned(),
            channels: BTreeMap::new(),
        }
    }
}
