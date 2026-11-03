//! Channels, formats, routing, files, context and the request middleware.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use oxide_config::Configuration;
use oxide_http::__private::axum::Router;
use oxide_http::__private::axum::middleware::from_fn;
use oxide_http::__private::axum::routing::get;
use oxide_http::testing::TestClient;
use oxide_log::{
    ChannelConfig, Context, Driver, Log, LogError, Logger, LoggingConfig, MemoryWriter,
    context_middleware, tracing,
};
use serde_json::Value;

/// `Log::info` and friends are single `tracing` callsites shared by these tests, and `tracing`
/// caches whether a callsite is enabled across all loggers installed at the time. Tests that go
/// through them take this lock so another test's logger can't hide their events.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn config(default: &str, channels: &[(&str, ChannelConfig)]) -> LoggingConfig {
    LoggingConfig {
        default: default.to_owned(),
        level: "info".to_owned(),
        channels: channels
            .iter()
            .map(|(name, channel)| ((*name).to_owned(), channel.clone()))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn channel(driver: Driver) -> ChannelConfig {
    ChannelConfig {
        driver,
        ..ChannelConfig::default()
    }
}

fn temp_dir(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "oxide-log-{name}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[tokio::test]
async fn text_lines_carry_fields_and_context() {
    let sink = MemoryWriter::new();
    let config = config("stderr", &[]);
    let (dispatch, _guard) = Logger::builder(&config)
        .writer("stderr", sink.clone())
        .build()
        .unwrap();
    let _default = tracing::dispatcher::set_default(&dispatch);

    Context::scope(async {
        Context::add("request_id", "req-1");
        Context::add_hidden("tenant", 7);
        tracing::info!(user_id = 42, name = "Ann Lee", "signed in");
    })
    .await;
    tracing::warn!("outside any context");

    let lines = sink.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("INFO"), "{}", lines[0]);
    assert!(lines[0].contains(": signed in"), "{}", lines[0]);
    assert!(
        lines[0].contains("user_id=42") && lines[0].contains("name=\"Ann Lee\""),
        "{}",
        lines[0]
    );
    assert!(lines[0].ends_with("[request_id=req-1]"), "{}", lines[0]);
    assert!(
        !lines[0].contains("tenant"),
        "hidden context stays out: {}",
        lines[0]
    );
    assert!(
        lines[1].contains("WARN") && !lines[1].contains('['),
        "{}",
        lines[1]
    );
}

#[tokio::test]
async fn the_json_driver_writes_one_object_per_line() {
    let sink = MemoryWriter::new();
    let config = config("json", &[("json", channel(Driver::Json))]);
    let (dispatch, _guard) = Logger::builder(&config)
        .writer("json", sink.clone())
        .build()
        .unwrap();
    let _default = tracing::dispatcher::set_default(&dispatch);

    Context::scope(async {
        Context::add("request_id", "req-2");
        tracing::error!(code = 500, ok = false, ratio = 0.5, "failed");
    })
    .await;

    let line: Value = serde_json::from_str(&sink.lines()[0]).unwrap();
    assert_eq!(line["level"], "ERROR");
    assert_eq!(line["message"], "failed");
    assert_eq!(
        line["fields"],
        serde_json::json!({ "code": 500, "ok": false, "ratio": 0.5 })
    );
    assert_eq!(
        line["context"],
        serde_json::json!({ "request_id": "req-2" })
    );
    assert!(line["timestamp"].as_str().unwrap().ends_with('Z'));
    assert!(line["target"].is_string());
}

#[test]
fn levels_filter_per_channel() {
    let loud = MemoryWriter::new();
    let quiet = MemoryWriter::new();
    let off = MemoryWriter::new();
    let mut debugging = channel(Driver::Stderr);
    debugging.level = Some("debug".into());
    let mut disabled = channel(Driver::Stderr);
    disabled.level = Some("off".into());
    let config = config(
        "everything",
        &[
            ("everything", channel(Driver::Stack)),
            ("loud", debugging),
            ("off", disabled),
        ],
    );
    let mut stack = config.clone();
    stack.channels.get_mut("everything").unwrap().channels =
        vec!["loud".into(), "stderr".into(), "off".into()];
    let (dispatch, _guard) = Logger::builder(&stack)
        .writer("loud", loud.clone())
        .writer("stderr", quiet.clone())
        .writer("off", off.clone())
        .build()
        .unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::debug!("detail");
        tracing::info!("news");
        tracing::trace!("noise");
    });
    assert_eq!(
        loud.lines().len(),
        2,
        "debug channel records debug and info"
    );
    assert_eq!(
        quiet.lines().len(),
        1,
        "the global level (info) hides debug"
    );
    assert_eq!(off.lines().len(), 0);
}

#[test]
fn addressed_events_go_only_to_their_channel() {
    let _serial = serial();
    let main = MemoryWriter::new();
    let audit = MemoryWriter::new();
    let config = config("stderr", &[("audit", channel(Driver::Json))]);
    let (dispatch, _guard) = Logger::builder(&config)
        .writer("stderr", main.clone())
        .writer("audit", audit.clone())
        .build()
        .unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        Log::info("ordinary");
        Log::channel("audit").warn("exported report");
        Log::channel("nowhere").error("falls back to the default");
        tracing::info!(channel = "audit", user = 7, "structured and addressed");
    });
    let main_lines = main.lines();
    assert_eq!(main_lines.len(), 2, "{main_lines:?}");
    assert!(main_lines[0].contains("ordinary") && main_lines[1].contains("falls back"));
    let audit_lines = audit.lines();
    assert_eq!(audit_lines.len(), 2, "{audit_lines:?}");
    let first: Value = serde_json::from_str(&audit_lines[0]).unwrap();
    assert_eq!(
        (first["level"].as_str(), first["channel"].as_str()),
        (Some("WARN"), Some("audit"))
    );
    let second: Value = serde_json::from_str(&audit_lines[1]).unwrap();
    assert_eq!(second["fields"]["user"], 7);
}

#[test]
fn every_log_level_has_a_helper() {
    let _serial = serial();
    let sink = MemoryWriter::new();
    let mut verbose = channel(Driver::Stderr);
    verbose.level = Some("trace".into());
    let config = config("v", &[("v", verbose)]);
    let (dispatch, _guard) = Logger::builder(&config)
        .writer("v", sink.clone())
        .build()
        .unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        Log::error("e");
        Log::warn("w");
        Log::info("i");
        Log::debug("d");
        Log::trace("t");
        Log::channel("v").trace("addressed trace");
    });
    let lines = sink.lines();
    assert_eq!(lines.len(), 6, "{lines:?}");
    for (line, level) in lines
        .iter()
        .zip(["ERROR", "WARN", "INFO", "DEBUG", "TRACE", "TRACE"])
    {
        assert!(line.contains(level), "{line} should be {level}");
    }
}

#[test]
fn bad_configuration_is_reported() {
    let unknown = config("missing", &[]);
    let error = Logger::builder(&unknown).build().unwrap_err();
    assert!(
        matches!(&error, LogError::UnknownChannel { name, .. } if name == "missing"),
        "{error}"
    );

    let mut broken = channel(Driver::Stack);
    broken.channels = vec!["ghost".into()];
    let error = Logger::builder(&config("stderr", &[("s", broken)]))
        .build()
        .unwrap_err();
    assert!(
        error.to_string().contains("`ghost`") && error.to_string().contains("stack `s`"),
        "{error}"
    );

    let mut a = channel(Driver::Stack);
    a.channels = vec!["b".into()];
    let mut b = channel(Driver::Stack);
    b.channels = vec!["a".into()];
    let error = Logger::builder(&config("a", &[("a", a), ("b", b)]))
        .build()
        .unwrap_err();
    assert!(
        matches!(&error, LogError::StackLoop(path) if path.contains("a -> b -> a")),
        "{error}"
    );

    let mut level = channel(Driver::Stderr);
    level.level = Some("loud".into());
    let error = Logger::builder(&config("l", &[("l", level)]))
        .build()
        .unwrap_err();
    assert!(
        error.to_string().contains("invalid level `loud`"),
        "{error}"
    );

    let mut bad_default = config("stderr", &[]);
    bad_default.level = "shouty".into();
    assert!(matches!(
        Logger::builder(&bad_default).build().unwrap_err(),
        LogError::InvalidLevel { .. }
    ));

    let pathless = channel(Driver::Single);
    let error = Logger::builder(&config("p", &[("p", pathless)]))
        .build()
        .unwrap_err();
    assert!(
        matches!(&error, LogError::MissingPath(name) if name == "p"),
        "{error}"
    );
}

#[test]
fn file_channels_write_through_a_background_worker() {
    let dir = temp_dir("files");
    let mut single = channel(Driver::Single);
    single.path = Some(dir.join("nested/app.log"));
    let mut daily = channel(Driver::Daily);
    daily.path = Some(dir.join("daily/app.log"));
    daily.days = Some(3);
    let mut json = channel(Driver::Json);
    json.path = Some(dir.join("audit.log"));
    let mut stack = channel(Driver::Stack);
    stack.channels = vec!["single".into(), "daily".into(), "json".into()];
    let config = config(
        "all",
        &[
            ("all", stack),
            ("single", single),
            ("daily", daily),
            ("json", json),
        ],
    );

    let (dispatch, guard) = Logger::builder(&config).build().unwrap();
    tracing::dispatcher::with_default(&dispatch, || tracing::info!(n = 1, "to the files"));
    drop(guard); // flushes the background writers

    let single = fs::read_to_string(dir.join("nested/app.log")).unwrap();
    assert!(
        single.contains("to the files") && single.contains("n=1"),
        "{single}"
    );
    let daily_files: Vec<_> = fs::read_dir(dir.join("daily"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let name = daily_files[0].to_string_lossy().into_owned();
    assert!(
        name.starts_with("app.20")
            && std::path::Path::new(&name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("log")),
        "{name}"
    );
    let daily = fs::read_to_string(dir.join("daily").join(&name)).unwrap();
    assert!(daily.contains("to the files"), "{daily}");
    let json: Value =
        serde_json::from_str(fs::read_to_string(dir.join("audit.log")).unwrap().trim()).unwrap();
    assert_eq!(json["message"], "to the files");
}

#[test]
fn an_unwritable_path_is_an_error() {
    let dir = temp_dir("blocked");
    fs::create_dir_all(&dir).unwrap();
    let blocker = dir.join("file");
    fs::write(&blocker, "x").unwrap();
    let mut single = channel(Driver::Single);
    single.path = Some(blocker.join("inside/app.log"));
    let error = Logger::builder(&config("s", &[("s", single)]))
        .build()
        .unwrap_err();
    assert!(matches!(error, LogError::Io { .. }), "{error}");
}

#[test]
fn logging_config_loads_from_toml_and_the_environment() {
    let root = temp_dir("config");
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(
        root.join("config/logging.toml"),
        "default = \"stack\"\n\n[channels.stack]\ndriver = \"stack\"\nchannels = [\"stderr\", \"audit\"]\n\n[channels.audit]\ndriver = \"json\"\npath = \"audit.log\"\nlevel = \"debug\"\n",
    )
    .unwrap();
    let configuration = Configuration::builder()
        .vars([("LOG_LEVEL", "warn")])
        .root(&root)
        .section::<LoggingConfig>()
        .load()
        .unwrap();
    let logging = configuration.get::<LoggingConfig>().unwrap();
    assert_eq!(
        (logging.default.as_str(), logging.level.as_str()),
        ("stack", "warn")
    );
    assert_eq!(logging.channels["audit"].driver, Driver::Json);
    assert_eq!(logging.channels["stack"].channels, ["stderr", "audit"]);

    // With no file at all the defaults log info and above to stderr.
    let empty = Configuration::builder()
        .vars(Vec::<(String, String)>::new())
        .root(temp_dir("empty"))
        .section::<LoggingConfig>()
        .load()
        .unwrap();
    let logging = empty.get::<LoggingConfig>().unwrap();
    assert_eq!(
        (logging.default.as_str(), logging.level.as_str()),
        ("stderr", "info")
    );
    assert!(Logger::builder(&logging).build().is_ok());
    assert_eq!(LoggingConfig::stderr("debug").level, "debug");
}

async fn handler() -> String {
    tracing::info!("handling");
    Context::get("request_id")
        .and_then(|id| id.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[tokio::test]
async fn the_middleware_adds_a_request_id_to_logs_and_responses() {
    let sink = MemoryWriter::new();
    let config = config("stderr", &[]);
    let (dispatch, _guard) = Logger::builder(&config)
        .writer("stderr", sink.clone())
        .build()
        .unwrap();
    let _default = tracing::dispatcher::set_default(&dispatch);

    let client = TestClient::new(
        Router::new()
            .route("/ping", get(handler))
            .layer(from_fn(context_middleware)),
    );

    // A fresh id is generated and returned.
    let response = client.get("/ping").send().await;
    response.assert_ok();
    let generated = response.text();
    assert_eq!(generated.len(), 36, "a UUID: {generated}");
    response.assert_header("x-request-id", &generated);
    assert!(
        sink.lines()[0].contains(&format!("request_id={generated}")),
        "{}",
        sink.lines()[0]
    );
    assert!(sink.lines()[0].contains("method=GET") && sink.lines()[0].contains("path=/ping"));

    // A sensible client id is kept...
    let response = client
        .get("/ping")
        .header("X-Request-Id", "trace-abc.123")
        .send()
        .await;
    assert_eq!(response.text(), "trace-abc.123");
    response.assert_header("x-request-id", "trace-abc.123");

    // ...and a hostile one is replaced.
    let response = client
        .get("/ping")
        .header("X-Request-Id", "bad id with spaces")
        .send()
        .await;
    assert_ne!(response.text(), "bad id with spaces");
    assert_eq!(response.text().len(), 36);
}

#[test]
fn a_stack_can_be_addressed_by_name() {
    let _serial = serial();
    let both = MemoryWriter::new();
    let other = MemoryWriter::new();
    let stderr = MemoryWriter::new();
    let mut team = channel(Driver::Stack);
    team.channels = vec!["a".into(), "b".into()];
    let config = config(
        "stderr",
        &[
            ("team", team),
            ("a", channel(Driver::Stderr)),
            ("b", channel(Driver::Json)),
            ("c", channel(Driver::Stderr)),
        ],
    );
    let (dispatch, _guard) = Logger::builder(&config)
        .writer("a", both.clone())
        .writer("b", other.clone())
        .writer("stderr", stderr.clone())
        .writer("c", MemoryWriter::new())
        .build()
        .unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        Log::channel("team").info("to the whole team");
        Log::channel("a").info("just a");
    });
    assert_eq!(both.lines().len(), 2, "{:?}", both.lines());
    assert_eq!(other.lines().len(), 1, "{:?}", other.lines());
    assert_eq!(
        stderr.lines().len(),
        0,
        "the default channel doesn't see addressed events"
    );
}

#[test]
fn logged_input_cannot_forge_lines() {
    let _serial = serial();
    let sink = MemoryWriter::new();
    let config = config("stderr", &[]);
    let (dispatch, _guard) = Logger::builder(&config)
        .writer("stderr", sink.clone())
        .build()
        .unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        Log::info(
            "login failed for x\n2026-01-01T00:00:00.000Z INFO  app: admin logged in\r\u{1b}[2J",
        );
    });
    let lines = sink.lines();
    assert_eq!(lines.len(), 1, "one event, one line: {lines:?}");
    assert!(lines[0].contains("x\\n2026-01-01"), "{}", lines[0]);
    assert!(lines[0].contains("\\r\\u{001b}[2J"), "{}", lines[0]);
}
