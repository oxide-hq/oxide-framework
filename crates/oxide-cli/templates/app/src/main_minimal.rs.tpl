use std::net::SocketAddr;

use {{crate_name}}::AppContext;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Values already in the environment win over `.env`; a missing `.env` is fine.
    dotenvy::dotenv().ok();
    let port = std::env::var("APP_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(3000);

    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    println!("{{name}} is listening on http://{}", listener.local_addr()?);
    oxide::axum::serve(listener, {{crate_name}}::build_app(AppContext::default())).await?;
    Ok(())
}
