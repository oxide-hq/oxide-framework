use std::net::SocketAddr;

use {{crate_name}}::config::AppConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Reads config/*.toml, `.env` and the environment, and reports every problem at once.
    let config = {{crate_name}}::config::load()?;
    let app = config.require::<AppConfig>()?;

    let ctx = {{crate_name}}::boot(config).await?;
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], app.port))).await?;
    println!("{} is listening on http://{}", app.name, listener.local_addr()?);
    oxide::axum::serve(listener, {{crate_name}}::build_app(ctx)).await?;
    Ok(())
}
