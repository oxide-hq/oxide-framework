//! How a log line looks: one text line, or one JSON object, always carrying the [`Context`].

use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::{FmtContext, MakeWriter};
use tracing_subscriber::registry::LookupSpan;

use crate::context::Context;

/// The channel an event is addressed to (see `Log::channel`); not shown in the output fields.
pub(crate) const CHANNEL_FIELD: &str = "channel";

/// Collects an event's fields into JSON.
#[derive(Default)]
pub(crate) struct FieldVisitor {
    pub(crate) fields: Map<String, Value>,
}

impl FieldVisitor {
    pub(crate) fn of(event: &Event<'_>) -> Self {
        let mut visitor = Self::default();
        event.record(&mut visitor);
        visitor
    }

    pub(crate) fn channel(&self) -> Option<&str> {
        self.fields.get(CHANNEL_FIELD).and_then(Value::as_str)
    }
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), Value::String(value.to_owned()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().to_owned(), value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_owned(), value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().to_owned(), value.into());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.fields.insert(field.name().to_owned(), value.into());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.fields
            .insert(field.name().to_owned(), Value::String(value.to_string()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), Value::String(format!("{value:?}")));
    }
}

/// The shape of a channel's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Text,
    Json,
}

pub(crate) struct ChannelFormat(pub(crate) Shape);

impl<S, N> FormatEvent<S, N> for ChannelFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut visitor = FieldVisitor::of(event);
        let message = visitor
            .fields
            .remove("message")
            .map(|value| match value {
                Value::String(text) => text,
                other => other.to_string(),
            })
            .unwrap_or_default();
        let channel = visitor.fields.remove(CHANNEL_FIELD);
        let context = Context::all();
        let metadata = event.metadata();
        let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

        match self.0 {
            Shape::Json => {
                let mut line = Map::new();
                line.insert("timestamp".into(), timestamp.into());
                line.insert("level".into(), metadata.level().as_str().into());
                line.insert("target".into(), metadata.target().into());
                line.insert("message".into(), message.into());
                if let Some(channel) = channel {
                    line.insert("channel".into(), channel);
                }
                if !visitor.fields.is_empty() {
                    line.insert("fields".into(), Value::Object(visitor.fields));
                }
                if !context.is_empty() {
                    line.insert(
                        "context".into(),
                        Value::Object(context.into_iter().collect()),
                    );
                }
                writeln!(writer, "{}", Value::Object(line))
            }
            Shape::Text => {
                write!(
                    writer,
                    "{timestamp} {:<5} {}: {message}",
                    metadata.level(),
                    metadata.target()
                )?;
                for (key, value) in &visitor.fields {
                    write!(writer, " {key}={}", plain(value))?;
                }
                if !context.is_empty() {
                    let pairs: Vec<String> = context
                        .iter()
                        .map(|(key, value)| format!("{key}={}", plain(value)))
                        .collect();
                    write!(writer, " [{}]", pairs.join(" "))?;
                }
                writeln!(writer)
            }
        }
    }
}

/// A value for a text line: bare when it is a simple word, JSON-quoted otherwise.
fn plain(value: &Value) -> String {
    match value {
        Value::String(text)
            if !text.is_empty() && !text.contains(char::is_whitespace) && !text.contains('"') =>
        {
            text.clone()
        }
        other => other.to_string(),
    }
}

/// A shared in-memory sink: capture log output in tests, or hand it to a channel.
#[derive(Debug, Clone, Default)]
pub struct MemoryWriter(Arc<Mutex<Vec<u8>>>);

impl MemoryWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything written so far.
    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap_or_else(PoisonError::into_inner)).into_owned()
    }

    /// The lines written so far.
    pub fn lines(&self) -> Vec<String> {
        self.contents().lines().map(str::to_owned).collect()
    }
}

impl io::Write for MemoryWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for MemoryWriter {
    type Writer = MemoryWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
