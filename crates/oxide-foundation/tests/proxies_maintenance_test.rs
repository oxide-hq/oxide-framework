//! `TrustProxies` and maintenance mode.

use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use oxide_foundation::{
    Application, Client, Down, PreventRequestsDuringMaintenance, TrustProxies, down, is_down,
    middleware_fn, up,
};
use oxide_http::__private::axum::Json;
use oxide_http::__private::axum::Router;
use oxide_http::__private::axum::extract::{ConnectInfo, Request};
use oxide_http::__private::axum::routing::get;
use oxide_http::testing::TestClient;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn whoami(Client(client): Client) -> Json<Value> {
    Json(json!({
        "ip": client.ip.map(|ip| ip.to_string()),
        "scheme": client.scheme,
        "host": client.host,
    }))
}

async fn hello() -> &'static str {
    "hello"
}

/// Pretends a connection came from the address in `X-Peer` (a real server supplies it).
fn peer_injector() -> impl oxide_foundation::Middleware {
    middleware_fn(
        |mut request: Request, next, _params: Vec<String>| async move {
            let peer = request
                .headers()
                .get("x-peer")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok());
            if let Some(ip) = peer {
                request
                    .extensions_mut()
                    .insert(ConnectInfo(SocketAddr::new(ip, 40_000)));
            }
            next.run(request).await
        },
    )
}

async fn proxied(proxies: TrustProxies) -> TestClient {
    Application::builder()
        .middleware(peer_injector())
        .middleware(proxies)
        .routes(Router::new().route("/whoami", get(whoami)))
        .build()
        .await
        .unwrap()
        .test_client()
}

#[tokio::test]
async fn forwarded_headers_are_believed_only_from_trusted_peers() {
    let client = proxied(TrustProxies::new(["10.0.0.0/8", "192.168.1.1"]).unwrap()).await;

    // A stranger can't forge them.
    let stranger = client
        .get("/whoami")
        .header("X-Peer", "203.0.113.9")
        .header("Host", "app.test")
        .header("X-Forwarded-For", "1.2.3.4")
        .header("X-Forwarded-Proto", "https")
        .header("X-Forwarded-Host", "evil.test")
        .send()
        .await
        .json();
    assert_eq!(
        stranger,
        json!({ "ip": "203.0.113.9", "scheme": "http", "host": "app.test" })
    );

    // A trusted proxy's headers are used: the client is the first address from the right that
    // isn't itself a trusted proxy.
    let proxied = client
        .get("/whoami")
        .header("X-Peer", "10.0.0.5")
        .header("Host", "internal")
        .header("X-Forwarded-For", "198.51.100.7, 10.0.0.9")
        .header("X-Forwarded-Proto", "HTTPS")
        .header("X-Forwarded-Host", "app.example.com")
        .send()
        .await
        .json();
    assert_eq!(
        proxied,
        json!({ "ip": "198.51.100.7", "scheme": "https", "host": "app.example.com" })
    );

    // Entries a client prepended on the left can't displace the real one.
    let spoofed = client
        .get("/whoami")
        .header("X-Peer", "192.168.1.1")
        .header("X-Forwarded-For", "6.6.6.6, 198.51.100.7")
        .send()
        .await
        .json();
    assert_eq!(spoofed["ip"], "198.51.100.7");

    // Junk is ignored rather than trusted: an unreadable address stops the walk, and only
    // http and https are accepted schemes.
    let junk = client
        .get("/whoami")
        .header("X-Peer", "10.0.0.5")
        .header("X-Forwarded-For", "not-an-ip")
        .header("X-Forwarded-Proto", "javascript")
        .send()
        .await
        .json();
    assert_eq!(
        (junk["ip"].as_str(), junk["scheme"].as_str()),
        (Some("10.0.0.5"), Some("http"))
    );

    // Without a peer address (no real connection) nothing is known.
    assert_eq!(client.get("/whoami").send().await.json()["ip"], Value::Null);
}

#[tokio::test]
async fn trusting_everyone_and_configuration_errors() {
    let client = proxied(TrustProxies::all()).await;
    let response = client
        .get("/whoami")
        .header("X-Peer", "203.0.113.9")
        .header("X-Forwarded-For", "198.51.100.7")
        .send()
        .await
        .json();
    assert_eq!(response["ip"], "198.51.100.7");

    assert!(
        TrustProxies::new(["10.0.0.0/8", "bogus"])
            .unwrap_err()
            .contains("bogus")
    );
    assert!(TrustProxies::new(["10.0.0.0/40"]).is_err());
}

#[tokio::test]
async fn the_client_extractor_needs_the_middleware() {
    let app = Application::builder()
        .routes(Router::new().route("/whoami", get(whoami)))
        .build()
        .await
        .unwrap();
    let response = app.test_client().get("/whoami").send().await;
    assert_eq!(response.status().as_u16(), 500);
    assert!(
        !response.text().contains("TrustProxies"),
        "{}",
        response.text()
    );
}

#[tokio::test]
async fn serve_gives_the_real_peer_address() {
    let app = Application::builder()
        .middleware(TrustProxies::new(["127.0.0.1"]).unwrap())
        .routes(Router::new().route("/whoami", get(whoami)))
        .build()
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(app.serve(listener));

    let ask = |forwarded: &'static str| async move {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let request = format!(
            "GET /whoami HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{forwarded}\r\n"
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        serde_json::from_str::<Value>(reply.rsplit("\r\n\r\n").next().unwrap()).unwrap()
    };
    // Loopback is a trusted proxy here, so its forwarded address is the client...
    assert_eq!(
        ask("X-Forwarded-For: 198.51.100.7\r\n").await["ip"],
        "198.51.100.7"
    );
    // ...and with no header the peer itself is.
    assert_eq!(ask("").await["ip"], "127.0.0.1");
    server.abort();
}

fn maintenance_file(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    std::env::temp_dir().join(format!(
        "oxide-maintenance-{name}-{}-{}/storage/framework/down",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

async fn guarded(file: &PathBuf) -> TestClient {
    Application::builder()
        .middleware(PreventRequestsDuringMaintenance::new(file).except(["/up", "/health*"]))
        .routes(
            Router::new()
                .route("/hello", get(hello))
                .route("/up", get(hello))
                .route("/health/db", get(hello)),
        )
        .build()
        .await
        .unwrap()
        .test_client()
}

#[tokio::test]
async fn maintenance_mode_answers_503_until_the_app_is_brought_up() {
    let file = maintenance_file("cycle");
    let client = guarded(&file).await;
    client.get("/hello").send().await.assert_ok();
    assert!(!is_down(&file));
    assert!(!up(&file).unwrap(), "already up");

    down(
        &file,
        &Down {
            retry: Some(60),
            message: Some("Back at noon.".into()),
            secret: Some("let-me-in".into()),
        },
    )
    .unwrap();
    assert!(is_down(&file));
    // The first check after a state change can be up to a second stale; wait it out.
    tokio::time::sleep(Duration::from_millis(1100)).await;

    let response = client.get("/hello").send().await;
    assert_eq!(response.status().as_u16(), 503);
    assert_eq!(response.json(), json!({ "message": "Back at noon." }));
    assert_eq!(response.headers().get("retry-after").unwrap(), "60");

    // Excepted paths, and requests with the secret, still work; a wrong secret doesn't.
    client.get("/up").send().await.assert_ok();
    client.get("/health/db").send().await.assert_ok();
    client
        .get("/hello")
        .header("X-Maintenance-Secret", "let-me-in")
        .send()
        .await
        .assert_ok();
    assert_eq!(
        client
            .get("/hello")
            .header("X-Maintenance-Secret", "nope")
            .send()
            .await
            .status()
            .as_u16(),
        503
    );

    assert!(up(&file).unwrap());
    tokio::time::sleep(Duration::from_millis(1100)).await;
    client.get("/hello").send().await.assert_ok();
}

#[tokio::test]
async fn an_unreadable_maintenance_file_still_means_down_with_defaults() {
    let file = maintenance_file("garbage");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "not json").unwrap();
    let client = guarded(&file).await;
    let response = client.get("/hello").send().await;
    assert_eq!(response.status().as_u16(), 503);
    assert_eq!(response.json(), json!({ "message": "Service Unavailable" }));
    assert!(response.headers().get("retry-after").is_none());
}
