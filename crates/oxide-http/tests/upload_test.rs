//! `Uploads` through a real route with a hand-built multipart body.

use axum::Json;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use oxide_http::prelude::*;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn store(uploads: Uploads) -> Result<Json<Value>, OxideHttpError> {
    let avatar = uploads
        .file("avatar")
        .ok_or_else(|| OxideHttpError::BadRequest("avatar required".into()))?;
    avatar.validate(
        "avatar",
        &FileRules::new().image().mimes(&["png"]).max_kb(1),
    )?;
    Ok(Json(json!({
        "name": avatar.name(),
        "size": avatar.size(),
        "title": uploads.field("title"),
        "photos": uploads.files("photos").len(),
    })))
}

/// `(field name, Some((file name, mime)) for files, data)`.
type Part<'a> = (&'a str, Option<(&'a str, &'a str)>, &'a [u8]);

async fn send(parts: &[Part<'_>]) -> (StatusCode, Value) {
    let mut body = Vec::new();
    for (name, file, data) in parts {
        body.extend(format!("--B\r\nContent-Disposition: form-data; name=\"{name}\"").as_bytes());
        if let Some((file_name, mime)) = file {
            body.extend(format!("; filename=\"{file_name}\"\r\nContent-Type: {mime}").as_bytes());
        }
        body.extend(b"\r\n\r\n");
        body.extend(*data);
        body.extend(b"\r\n");
    }
    body.extend(b"--B--\r\n");
    let router: OxideRouter<()> = routes! { post("/up", store); };
    let response = axum::Router::from(router)
        .oneshot(
            Request::post("/up")
                .header("content-type", "multipart/form-data; boundary=B")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn files_and_fields_are_parsed_and_validated() {
    let (status, body) = send(&[
        ("title", None, b"Hi"),
        ("avatar", Some(("a.png", "image/png")), b"123"),
        ("photos[]", Some(("1.png", "image/png")), b"x"),
        ("photos[]", Some(("2.png", "image/png")), b"y"),
    ])
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({ "name": "a.png", "size": 3, "title": "Hi", "photos": 2 })
    );

    let (status, _) = send(&[("avatar", Some(("a.txt", "text/plain")), b"123")]).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, _) = send(&[("title", None, b"only text")]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
