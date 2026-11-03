//! Installing the logger for the process (one test, because it is process-global).

use oxide_log::{LogError, Logger, LoggingConfig, MemoryWriter, tracing};

#[test]
fn init_installs_once_and_bridges_the_log_crate() {
    let sink = MemoryWriter::new();
    let config = LoggingConfig::stderr("info");
    let guard = Logger::builder(&config)
        .writer("stderr", sink.clone())
        .init()
        .unwrap();

    tracing::info!("the global logger works");
    // A dependency that uses the `log` crate still reaches the channels.
    log::warn!(target: "some_dependency", "from the log crate");

    let lines = sink.lines();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("the global logger works"), "{}", lines[0]);
    assert!(
        lines[1].contains("WARN") && lines[1].contains("from the log crate"),
        "{}",
        lines[1]
    );
    drop(guard);
    assert!(matches!(
        Logger::init(&config),
        Err(LogError::AlreadyInitialized)
    ));
}
