//! Installing the logger for the process (one test, because it is process-global).

use oxide_log::{LogError, Logger, LoggingConfig, tracing};

#[test]
fn init_installs_once() {
    let config = LoggingConfig::stderr("info");
    let guard = Logger::init(&config).unwrap();
    tracing::info!("the global logger works");
    drop(guard);
    assert!(matches!(
        Logger::init(&config),
        Err(LogError::AlreadyInitialized)
    ));
}
