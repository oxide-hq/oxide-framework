use std::net::SocketAddr;

use oxide::prelude::{Logger, LoggingConfig};
use {{crate_name}}::config::AppConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Reads config/*.toml, `.env` and the environment, and reports every problem at once.
    let config = {{crate_name}}::config::load()?;
    let app = config.require::<AppConfig>()?;
    // Keep the guard alive: dropping it flushes and stops file logging.
    let logging = config.require::<LoggingConfig>()?;
    let _logs = Logger::init(&logging)?;

    let ctx = {{crate_name}}::boot(config).await?;
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], app.port))).await?;
    println!("{} is listening on http://{}", app.name, listener.local_addr()?);
    oxide::axum::serve(listener, {{crate_name}}::build_app(ctx).await?).await?;
    Ok(())
}
