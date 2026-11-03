//! Turning a [`LoggingConfig`] into a `tracing` subscriber.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use tracing::{Dispatch, Event, Level, Metadata};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::Registry;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::layer::{Context as LayerContext, Filter, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

use crate::config::{ChannelConfig, Driver, LoggingConfig};
use crate::format::{ChannelFormat, FieldVisitor, Shape};

/// Why logging couldn't be set up.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("logging channel `{name}` is not defined (used by `{used_by}`)")]
    UnknownChannel { name: String, used_by: String },
    #[error("logging stack loops back on itself: {0}")]
    StackLoop(String),
    #[error(
        "channel `{channel}` has an invalid level `{level}`; use error, warn, info, debug, trace or off"
    )]
    InvalidLevel { channel: String, level: String },
    #[error("channel `{0}` needs a `path`")]
    MissingPath(String),
    #[error("could not open the log file `{path}`: {message}")]
    Io { path: PathBuf, message: String },
    #[error("a logger was already installed in this process")]
    AlreadyInitialized,
}

/// Keeps the background writers of file channels alive and flushes them when dropped. Hold it
/// for the life of the program (`let _logs = Logger::init(..)?;` in `main`).
#[must_use = "dropping the guard stops file logging; keep it for the life of the program"]
#[derive(Debug)]
pub struct LogGuard {
    _workers: Vec<WorkerGuard>,
}

/// Builds loggers from configuration.
#[derive(Debug, Clone, Copy)]
pub struct Logger;

impl Logger {
    /// Installs the logger for the whole process.
    pub fn init(config: &LoggingConfig) -> Result<LogGuard, LogError> {
        Self::builder(config).init()
    }

    pub fn builder(config: &LoggingConfig) -> LoggerBuilder<'_> {
        LoggerBuilder {
            config,
            writers: HashMap::new(),
        }
    }
}

/// Configures a logger before it is built; see [`Logger::builder`].
#[must_use]
pub struct LoggerBuilder<'a> {
    config: &'a LoggingConfig,
    writers: HashMap<String, BoxMakeWriter>,
}

impl std::fmt::Debug for LoggerBuilder<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoggerBuilder")
            .field("overridden", &self.writers.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl LoggerBuilder<'_> {
    /// Sends channel `name` to `writer` instead of where its driver says, for example a
    /// [`MemoryWriter`](crate::MemoryWriter) in tests.
    pub fn writer(
        mut self,
        name: impl Into<String>,
        writer: impl for<'w> MakeWriter<'w> + Send + Sync + 'static,
    ) -> Self {
        self.writers.insert(name.into(), BoxMakeWriter::new(writer));
        self
    }

    /// Builds the subscriber without installing it. Use `tracing::dispatcher::set_default` for
    /// a scoped logger in a test.
    pub fn build(self) -> Result<(Dispatch, LogGuard), LogError> {
        let plan = Plan::of(self.config)?;
        let known = Arc::new(plan.known.clone());
        let mut writers = self.writers;
        let mut workers = Vec::new();
        let mut layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> = Vec::new();
        for leaf in &plan.leaves {
            let writer = match writers.remove(&leaf.name) {
                Some(writer) => writer,
                None => open(leaf, &mut workers)?,
            };
            let shape = if leaf.driver == Driver::Json {
                Shape::Json
            } else {
                Shape::Text
            };
            let filter = ChannelFilter {
                name: leaf.name.clone(),
                in_default: plan.default_leaves.contains(&leaf.name),
                level: leaf.level,
                known: Arc::clone(&known),
            };
            layers.push(
                tracing_subscriber::fmt::layer()
                    .event_format(ChannelFormat(shape))
                    .with_writer(writer)
                    .with_ansi(false)
                    .with_filter(filter)
                    .boxed(),
            );
        }
        let subscriber = Registry::default().with(layers);
        Ok((Dispatch::new(subscriber), LogGuard { _workers: workers }))
    }

    /// Builds the logger and installs it for the whole process.
    pub fn init(self) -> Result<LogGuard, LogError> {
        let (dispatch, guard) = self.build()?;
        tracing::dispatcher::set_global_default(dispatch)
            .map_err(|_| LogError::AlreadyInitialized)?;
        Ok(guard)
    }
}

struct Leaf {
    name: String,
    driver: Driver,
    path: Option<PathBuf>,
    /// `None` is `off`.
    level: Option<Level>,
    days: Option<usize>,
}

struct Plan {
    leaves: Vec<Leaf>,
    default_leaves: HashSet<String>,
    known: HashSet<String>,
}

impl Plan {
    fn of(config: &LoggingConfig) -> Result<Self, LogError> {
        let mut channels: BTreeMap<&str, ChannelConfig> = config
            .channels
            .iter()
            .map(|(name, channel)| (name.as_str(), channel.clone()))
            .collect();
        channels.entry("stderr").or_default();

        let global = parse_level("logging", &config.level)?;
        let mut leaves = Vec::new();
        for (name, channel) in &channels {
            if channel.driver == Driver::Stack {
                continue;
            }
            let level = match &channel.level {
                Some(level) => parse_level(name, level)?,
                None => global,
            };
            leaves.push(Leaf {
                name: (*name).to_owned(),
                driver: channel.driver,
                path: channel.path.clone(),
                level,
                days: channel.days,
            });
        }

        // Every stack must name real channels and never loop, whether or not it is the default.
        for name in channels.keys() {
            expand(name, "the configuration", &channels, &mut Vec::new())?;
        }
        let default_leaves = expand(&config.default, "`default`", &channels, &mut Vec::new())?
            .into_iter()
            .collect();
        Ok(Self {
            leaves,
            default_leaves,
            known: channels.keys().map(|name| (*name).to_owned()).collect(),
        })
    }
}

/// The leaf channels `name` stands for: itself, or the members of a stack.
fn expand(
    name: &str,
    used_by: &str,
    channels: &BTreeMap<&str, ChannelConfig>,
    path: &mut Vec<String>,
) -> Result<Vec<String>, LogError> {
    let Some(channel) = channels.get(name) else {
        return Err(LogError::UnknownChannel {
            name: name.to_owned(),
            used_by: used_by.to_owned(),
        });
    };
    if channel.driver != Driver::Stack {
        return Ok(vec![name.to_owned()]);
    }
    if path.iter().any(|seen| seen == name) {
        path.push(name.to_owned());
        return Err(LogError::StackLoop(path.join(" -> ")));
    }
    path.push(name.to_owned());
    let mut leaves = Vec::new();
    for member in &channel.channels {
        leaves.extend(expand(member, &format!("stack `{name}`"), channels, path)?);
    }
    path.pop();
    Ok(leaves)
}

fn parse_level(channel: &str, level: &str) -> Result<Option<Level>, LogError> {
    if level.eq_ignore_ascii_case("off") {
        return Ok(None);
    }
    Level::from_str(level)
        .map(Some)
        .map_err(|_| LogError::InvalidLevel {
            channel: channel.to_owned(),
            level: level.to_owned(),
        })
}

/// The writer for a channel: standard error, or a file written on a background thread.
fn open(leaf: &Leaf, workers: &mut Vec<WorkerGuard>) -> Result<BoxMakeWriter, LogError> {
    let needs_file = matches!(leaf.driver, Driver::Single | Driver::Daily)
        || (leaf.driver == Driver::Json && leaf.path.is_some());
    if !needs_file {
        return Ok(BoxMakeWriter::new(io::stderr));
    }
    let path = leaf
        .path
        .as_ref()
        .ok_or_else(|| LogError::MissingPath(leaf.name.clone()))?;
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let io_error = |message: String| LogError::Io {
        path: path.clone(),
        message,
    };
    fs::create_dir_all(directory).map_err(|error| io_error(error.to_string()))?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| io_error("the path has no file name".to_owned()))?;

    let appender = if leaf.driver == Driver::Daily {
        let stem = path.file_stem().map_or_else(
            || file_name.clone(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        let mut builder = RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix(stem);
        if let Some(extension) = path.extension() {
            builder = builder.filename_suffix(extension.to_string_lossy().into_owned());
        }
        if let Some(days) = leaf.days.filter(|days| *days > 0) {
            builder = builder.max_log_files(days);
        }
        builder
            .build(directory)
            .map_err(|error| io_error(error.to_string()))?
    } else {
        RollingFileAppender::builder()
            .rotation(Rotation::NEVER)
            .filename_prefix(file_name)
            .build(directory)
            .map_err(|error| io_error(error.to_string()))?
    };
    let (writer, worker) = tracing_appender::non_blocking(appender);
    workers.push(worker);
    Ok(BoxMakeWriter::new(writer))
}

/// Decides whether a channel records an event: by level, then by who the event is addressed to.
struct ChannelFilter {
    name: String,
    in_default: bool,
    level: Option<Level>,
    known: Arc<HashSet<String>>,
}

impl<S> Filter<S> for ChannelFilter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn enabled(&self, meta: &Metadata<'_>, _cx: &LayerContext<'_, S>) -> bool {
        self.level.is_some_and(|level| *meta.level() <= level)
    }

    fn event_enabled(&self, event: &Event<'_>, _cx: &LayerContext<'_, S>) -> bool {
        match FieldVisitor::of(event).channel() {
            // Addressed to a channel that exists: only that channel records it.
            Some(addressed) if self.known.contains(addressed) => addressed == self.name,
            // Unaddressed, or addressed to nothing configured: the default channels do.
            _ => self.in_default,
        }
    }
}
