//! Configuration loading: files, environment variables, `.env` files, errors and extraction.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use oxide_config::{Config, ConfigBuilder, ConfigSection, Configuration, Environment, config_of};
use oxide_http::testing::TestClient;
use oxide_http::{ServiceContainer, facade_middleware};
use serde::Deserialize;
use validator::Validate;

#[allow(dead_code)]
#[derive(Clone, Deserialize, Config)]
#[config(file = "app")]
struct AppConfig {
    name: String,
    #[config(env = "OXIDE_CFG_TEST_PORT", default = 3000)]
    port: u16,
    #[config(env = "OXIDE_CFG_TEST_DEBUG", default = false)]
    debug: bool,
    #[config(env = "OXIDE_CFG_TEST_KEY", secret)]
    key: String,
    #[config(env = "OXIDE_CFG_TEST_HOSTS")]
    hosts: Option<Vec<String>>,
}

#[derive(Clone, Deserialize, Validate, Config)]
#[config(file = "mail", validate)]
struct MailConfig {
    #[validate(range(min = 1, max = 65535, message = "port must be between 1 and 65535"))]
    #[config(env = "OXIDE_CFG_TEST_MAIL_PORT", default = 25)]
    port: u16,
}

#[allow(dead_code)]
#[derive(Clone, Deserialize, Config)]
#[config(file = "tokens")]
struct Tokens {
    #[config(env = "OXIDE_CFG_TEST_TOKEN", secret)]
    limit: u8,
}

/// A fresh project directory holding `files` (relative path, contents).
fn project(files: &[(&str, &str)]) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "oxide-config-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    for (path, contents) in files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_vars() -> ConfigBuilder {
    Configuration::builder().vars(Vec::<(String, String)>::new())
}

const APP_TOML: &str = "name = \"Oxide\"\nport = 8000\nkey = \"from-file\"\n";

#[test]
fn file_values_beat_defaults() {
    let root = project(&[("config/app.toml", APP_TOML)]);
    let config = no_vars().root(&root).section::<AppConfig>().load().unwrap();
    let app = config.get::<AppConfig>().unwrap();
    assert_eq!(
        (app.name.as_str(), app.port, app.debug),
        ("Oxide", 8000, false)
    );
    assert!(app.hosts.is_none());
}

#[test]
fn environment_beats_file_and_is_coerced() {
    let root = project(&[("config/app.toml", APP_TOML)]);
    let config = no_vars()
        .vars([
            ("OXIDE_CFG_TEST_PORT", "9090"),
            ("OXIDE_CFG_TEST_DEBUG", "yes"),
            ("OXIDE_CFG_TEST_HOSTS", "a.test, b.test"),
        ])
        .root(&root)
        .section::<AppConfig>()
        .load()
        .unwrap();
    let app = config.get::<AppConfig>().unwrap();
    assert_eq!((app.port, app.debug), (9090, true));
    assert_eq!(
        app.hosts.as_deref(),
        Some(&["a.test".to_owned(), "b.test".to_owned()][..])
    );
}

#[test]
fn a_section_can_come_entirely_from_the_environment() {
    let root = project(&[]);
    let config = no_vars()
        .vars([("OXIDE_CFG_TEST_KEY", "k")])
        .root(&root)
        .section::<MailConfig>()
        .load()
        .unwrap();
    assert_eq!(config.get::<MailConfig>().unwrap().port, 25);
}

#[test]
fn every_problem_is_reported_at_once() {
    let root = project(&[("config/mail.toml", "port = 0\n")]);
    let errors = no_vars()
        .vars([
            ("OXIDE_CFG_TEST_PORT", "not-a-number"),
            ("OXIDE_CFG_TEST_DEBUG", "maybe"),
        ])
        .root(&root)
        .section::<AppConfig>()
        .section::<MailConfig>()
        .load()
        .unwrap_err();
    let text = errors.to_string();
    assert_eq!(errors.issues.len(), 3, "{text}");
    assert!(
        text.contains("[app.port]") && text.contains("$OXIDE_CFG_TEST_PORT"),
        "{text}"
    );
    assert!(
        text.contains("expected an integer (u16), got `not-a-number`"),
        "{text}"
    );
    assert!(
        text.contains("[app.debug]") && text.contains("expected a boolean"),
        "{text}"
    );
    assert!(
        text.contains("[mail] port: port must be between 1 and 65535"),
        "{text}"
    );
}

#[test]
fn missing_required_values_name_the_field_and_the_file() {
    let root = project(&[("config/app.toml", "port = 1\n")]);
    let errors = no_vars()
        .root(&root)
        .section::<AppConfig>()
        .load()
        .unwrap_err();
    let text = errors.to_string();
    assert!(text.contains("missing field `name`"), "{text}");
    assert!(text.contains("config/app.toml"), "{text}");

    let errors = no_vars()
        .root(project(&[]))
        .section::<AppConfig>()
        .load()
        .unwrap_err();
    assert!(errors.to_string().contains("environment and defaults"));
}

#[test]
fn malformed_toml_is_an_issue_not_a_panic() {
    let root = project(&[("config/app.toml", "name = \n")]);
    let errors = no_vars()
        .root(&root)
        .section::<AppConfig>()
        .load()
        .unwrap_err();
    assert!(errors.to_string().contains("config/app.toml"));
}

#[test]
fn secrets_stay_out_of_debug_and_errors() {
    let root = project(&[("config/app.toml", "name = \"Oxide\"\n")]);
    let config = no_vars()
        .vars([("OXIDE_CFG_TEST_KEY", "hunter2-secret")])
        .root(&root)
        .section::<AppConfig>()
        .load()
        .unwrap();
    let shown = format!("{:?}", config.get::<AppConfig>().unwrap());
    assert!(
        shown.contains("[REDACTED]") && !shown.contains("hunter2"),
        "{shown}"
    );
    assert!(!format!("{config:?}").contains("hunter2"));

    // A secret that fails to coerce never echoes its value.
    let errors = no_vars()
        .vars([("OXIDE_CFG_TEST_TOKEN", "999-hunter3")])
        .root(project(&[]))
        .section::<Tokens>()
        .load()
        .unwrap_err();
    assert!(!errors.to_string().contains("hunter3"), "{errors}");
}

#[test]
fn dotenv_files_feed_the_environment_and_the_process_wins() {
    let root = project(&[
        ("config/app.toml", "name = \"Oxide\"\nkey = \"k\"\n"),
        (
            ".env",
            "OXIDE_CFG_TEST_PORT=4000\nOXIDE_CFG_TEST_DEBUG=true\n",
        ),
    ]);
    let config = Configuration::builder()
        .root(&root)
        .section::<AppConfig>()
        .load()
        .unwrap();
    let app = config.get::<AppConfig>().unwrap();
    assert_eq!((app.port, app.debug), (4000, true));
    assert_eq!(config.env("OXIDE_CFG_TEST_PORT"), Some("4000"));
    assert!(config.environment().is_local());
}

#[test]
fn the_environment_file_replaces_dotenv() {
    let root = project(&[
        ("config/app.toml", "name = \"Oxide\"\nkey = \"k\"\n"),
        (
            ".env",
            "OXIDE_CFG_TEST_PORT=4000\nOXIDE_CFG_TEST_DEBUG=true\n",
        ),
        (".env.testing", "OXIDE_CFG_TEST_PORT=5000\n"),
    ]);
    let config = Configuration::builder()
        .root(&root)
        .environment(Environment::Testing)
        .section::<AppConfig>()
        .load()
        .unwrap();
    let app = config.get::<AppConfig>().unwrap();
    // `.env.testing` replaces `.env` entirely, so `debug` falls back to its default.
    assert_eq!((app.port, app.debug), (5000, false));
    assert!(config.environment().is_testing());

    // `APP_ENV` in `.env` picks the environment file too.
    let root = project(&[
        ("config/app.toml", "name = \"Oxide\"\nkey = \"k\"\n"),
        (".env", "APP_ENV=staging\nOXIDE_CFG_TEST_PORT=4000\n"),
        (".env.staging", "OXIDE_CFG_TEST_PORT=6000\n"),
    ]);
    let config = Configuration::builder()
        .root(&root)
        .section::<AppConfig>()
        .load()
        .unwrap();
    assert_eq!(config.get::<AppConfig>().unwrap().port, 6000);
    assert_eq!(config.environment(), &Environment::Staging);
}

#[test]
fn a_broken_dotenv_is_reported() {
    let root = project(&[(".env", "NOT A VALID LINE\n")]);
    let errors = Configuration::builder().root(&root).load().unwrap_err();
    assert!(errors.to_string().contains(".env"), "{errors}");
}

#[test]
fn provided_sections_skip_loading() {
    let config = no_vars().provide(MailConfig { port: 2525 }).load().unwrap();
    assert_eq!(config.get::<MailConfig>().unwrap().port, 2525);
    assert!(config.get::<AppConfig>().is_none());
    assert_eq!(MailConfig::FILE, "mail");
    assert_eq!(MailConfig::fields().len(), 1);
}

#[derive(Clone, Debug, oxide_http::ServiceContainer)]
struct State {
    configuration: Configuration,
}

async fn show(Config(app): Config<AppConfig>) -> String {
    format!("{}:{}", app.name, app.port)
}

async fn show_mail(Config(mail): Config<MailConfig>) -> String {
    mail.port.to_string()
}

async fn facade() -> String {
    config_of::<State, AppConfig>().name.clone()
}

#[tokio::test]
async fn handlers_extract_sections_and_facades_resolve_them() {
    let configuration = no_vars()
        .provide(AppConfig {
            name: "Oxide".into(),
            port: 80,
            debug: false,
            key: "k".into(),
            hosts: None,
        })
        .load()
        .unwrap();
    let state = State { configuration };
    let router = oxide_http::__private::axum::Router::new()
        .route("/", oxide_http::__private::axum::routing::get(show))
        .route(
            "/mail",
            oxide_http::__private::axum::routing::get(show_mail),
        )
        .route("/facade", oxide_http::__private::axum::routing::get(facade))
        .layer(oxide_http::__private::axum::middleware::from_fn_with_state(
            state.clone(),
            facade_middleware::<State>,
        ))
        .with_state(state);

    let client = TestClient::new(router);
    let response = client.get("/").send().await;
    response.assert_ok();
    assert_eq!(response.text(), "Oxide:80");
    let response = client.get("/facade").send().await;
    response.assert_ok();
    assert_eq!(response.text(), "Oxide");
    // A section nobody registered is a server error, not a panic.
    assert_eq!(client.get("/mail").send().await.status().as_u16(), 500);
}

#[test]
fn facades_outside_a_scope_panic_with_a_hint() {
    let result = std::panic::catch_unwind(config_of::<State, AppConfig>);
    assert!(result.is_err());
    assert!(State::try_current().is_none());
}
