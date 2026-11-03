//! Response builders, like Laravel's `response()` and `redirect()` helpers.
//!
//! ```ignore
//! use oxide_http::response::{self, Cookie, ResponseExt};
//!
//! async fn export() -> impl IntoResponse {
//!     response::download("storage/report.csv", "report.csv").await
//! }
//! async fn created() -> Response {
//!     response::json(json!({ "ok": true }))
//!         .with_status(StatusCode::CREATED)
//!         .with_cookie(Cookie::new("seen", "1").http_only())
//! }
//! ```

use std::fmt::Write as _;
use std::path::Path;

use axum::body::{Body, Bytes};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE, LOCATION, REFERER, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_core::TryStream;
use serde::Serialize;

pub use axum::response::sse::{Event, KeepAlive, Sse};

use crate::error::OxideHttpError;
use crate::routing::UrlGenerator;

/// A JSON response with status 200.
pub fn json<T: Serialize>(value: T) -> Response {
    axum::Json(value).into_response()
}

/// `204 No Content`.
pub fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

/// Streams `stream` as the body, chunk by chunk.
pub fn stream<S>(stream: S) -> Response
where
    S: TryStream + Send + 'static,
    S::Ok: Into<Bytes>,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    Response::new(Body::from_stream(stream))
}

/// Serves a file inline (read fully into memory; stream big files with [`stream`]), with a content type guessed from its extension.
pub async fn file(path: impl AsRef<Path>) -> Result<Response, OxideHttpError> {
    let path = path.as_ref();
    let bytes = read(path).await?;
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, content_type(path));
    Ok(response)
}

/// Serves a file as an attachment the browser saves under `name`.
pub async fn download(path: impl AsRef<Path>, name: &str) -> Result<Response, OxideHttpError> {
    let mut response = file(path).await?;
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '"' | '\\' | '/') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let encoded = percent_encoding::utf8_percent_encode(&safe, percent_encoding::NON_ALPHANUMERIC);
    let ascii: String = safe
        .chars()
        .map(|c| if c.is_ascii() { c } else { '_' })
        .collect();
    let value = format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}");
    if let Ok(value) = HeaderValue::from_str(&value) {
        response.headers_mut().insert(CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

async fn read(path: &Path) -> Result<Vec<u8>, OxideHttpError> {
    tokio::fs::read(path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            OxideHttpError::NotFound(format!("File not found: {}", path.display()))
        } else {
            OxideHttpError::Server {
                message: "Could not read file".to_owned(),
                source: Box::new(error),
            }
        }
    })
}

fn content_type(path: &Path) -> HeaderValue {
    let mime = match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("csv") => "text/csv; charset=utf-8",
        Some("txt" | "md") => "text/plain; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    };
    HeaderValue::from_static(mime)
}

/// Redirect builders: `redirect::to("/home")`, `redirect::back(&parts)`.
pub mod redirect {
    use super::{
        HeaderValue, IntoResponse, LOCATION, OxideHttpError, Parts, REFERER, Response, StatusCode,
        UrlGenerator,
    };

    fn with_status(location: &str, status: StatusCode) -> Response {
        let mut response = status.into_response();
        let value = HeaderValue::from_str(location).unwrap_or(HeaderValue::from_static("/"));
        response.headers_mut().insert(LOCATION, value);
        response
    }

    /// `302 Found` to a path on this application. Anything that isn't a plain path (a scheme,
    /// `//host`, a backslash) falls back to `/`; use [`away`] for other sites.
    pub fn to(path: &str) -> Response {
        let local = path.starts_with('/') && !path.starts_with("//") && !path.contains('\\');
        with_status(if local { path } else { "/" }, StatusCode::FOUND)
    }

    /// `302 Found` to a named route.
    pub fn route(
        urls: &UrlGenerator,
        name: &str,
        params: &[(&str, &str)],
    ) -> Result<Response, OxideHttpError> {
        Ok(with_status(&urls.url(name, params)?, StatusCode::FOUND))
    }

    /// `302 Found` to the page the request came from (its `Referer`), else `fallback`. Only a
    /// referer on the same host is followed.
    pub fn back(request: &Parts, fallback: &str) -> Response {
        let host = request
            .headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok());
        let referer = request.headers.get(REFERER).and_then(|v| v.to_str().ok());
        let target = referer
            .zip(host)
            .and_then(|(referer, host)| {
                let rest = referer.split_once("://")?.1;
                let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
                (authority == host).then(|| format!("/{path}"))
            })
            .unwrap_or_else(|| fallback.to_owned());
        to(&target)
    }

    /// `302 Found` to any URL, including other sites. Never pass user input here unchecked.
    pub fn away(url: &str) -> Response {
        with_status(url, StatusCode::FOUND)
    }

    /// `301 Moved Permanently`.
    pub fn permanent(path: &str) -> Response {
        let mut response = to(path);
        *response.status_mut() = StatusCode::MOVED_PERMANENTLY;
        response
    }
}

/// A cookie's `SameSite` policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

/// A `Set-Cookie` value. Plain only; signed and encrypted cookies arrive with sessions.
#[derive(Debug, Clone)]
pub struct Cookie {
    name: String,
    value: String,
    path: String,
    max_age: Option<u64>,
    http_only: bool,
    secure: bool,
    same_site: SameSite,
}

impl Cookie {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            path: "/".to_owned(),
            max_age: None,
            http_only: false,
            secure: false,
            same_site: SameSite::Lax,
        }
    }

    #[must_use]
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    #[must_use]
    pub fn max_age(mut self, seconds: u64) -> Self {
        self.max_age = Some(seconds);
        self
    }

    #[must_use]
    pub fn http_only(mut self) -> Self {
        self.http_only = true;
        self
    }

    #[must_use]
    pub fn secure(mut self) -> Self {
        self.secure = true;
        self
    }

    /// `SameSite=None` also sets `Secure`, which browsers require.
    #[must_use]
    pub fn same_site(mut self, policy: SameSite) -> Self {
        self.same_site = policy;
        self
    }

    /// A cookie that tells the browser to drop `name`.
    pub fn forget(name: impl Into<String>) -> Self {
        Self::new(name, "").max_age(0)
    }

    fn render(&self) -> String {
        use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
        let mut out = format!(
            "{}={}; Path={}; SameSite={}",
            utf8_percent_encode(&self.name, NON_ALPHANUMERIC),
            utf8_percent_encode(&self.value, NON_ALPHANUMERIC),
            self.path.replace(|c: char| c == ';' || c.is_control(), ""),
            match self.same_site {
                SameSite::Strict => "Strict",
                SameSite::Lax => "Lax",
                SameSite::None => "None",
            }
        );
        if let Some(age) = self.max_age {
            let _ = write!(out, "; Max-Age={age}");
        }
        if self.http_only {
            out.push_str("; HttpOnly");
        }
        if self.secure || self.same_site == SameSite::None {
            out.push_str("; Secure");
        }
        out
    }
}

/// Chainable tweaks on a finished [`Response`]. Add your own with an extension trait:
/// `trait MyExt { fn cached(self) -> Self; } impl MyExt for Response { .. }`.
pub trait ResponseExt: Sized {
    #[must_use]
    fn with_status(self, status: StatusCode) -> Self;
    #[must_use]
    fn with_header(self, name: HeaderName, value: &str) -> Self;
    #[must_use]
    fn with_headers<'a>(self, headers: impl IntoIterator<Item = (HeaderName, &'a str)>) -> Self;
    #[must_use]
    fn with_cookie(self, cookie: Cookie) -> Self;
}

impl ResponseExt for Response {
    fn with_status(mut self, status: StatusCode) -> Self {
        *self.status_mut() = status;
        self
    }

    fn with_header(mut self, name: HeaderName, value: &str) -> Self {
        if let Ok(value) = HeaderValue::from_str(value) {
            self.headers_mut().insert(name, value);
        }
        self
    }

    fn with_headers<'a>(self, headers: impl IntoIterator<Item = (HeaderName, &'a str)>) -> Self {
        headers.into_iter().fold(self, |response, (name, value)| {
            response.with_header(name, value)
        })
    }

    fn with_cookie(mut self, cookie: Cookie) -> Self {
        if let Ok(value) = HeaderValue::from_str(&cookie.render()) {
            self.headers_mut().append(SET_COOKIE, value);
        }
        self
    }
}

/// Types that know how to become a response, optionally looking at the request: a domain type
/// can answer JSON to API clients and redirect browsers.
pub trait Responsable {
    fn to_response(self, request: &Parts) -> Response;
}

/// Renders `value` for `request` (a [`Parts`] extracted in the handler).
pub fn respond<R: Responsable>(value: R, request: &Parts) -> Response {
    value.to_response(request)
}

#[cfg(test)]
mod tests {
    use axum::http::Request as HttpRequest;
    use http_body_util::BodyExt;

    use super::*;

    fn parts(headers: &[(&str, &str)]) -> Parts {
        let mut builder = HttpRequest::builder().uri("/");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(()).unwrap().into_parts().0
    }

    async fn text(response: Response) -> String {
        String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap()
    }

    #[test]
    fn redirects_stay_local_unless_away() {
        assert_eq!(redirect::to("/home").headers()[LOCATION], "/home");
        assert_eq!(redirect::to("https://evil.test").headers()[LOCATION], "/");
        assert_eq!(redirect::to("//evil.test").headers()[LOCATION], "/");
        assert_eq!(
            redirect::away("https://ok.test").headers()[LOCATION],
            "https://ok.test"
        );
        assert_eq!(
            redirect::permanent("/x").status(),
            StatusCode::MOVED_PERMANENTLY
        );
    }

    #[test]
    fn back_follows_only_same_host_referers() {
        let same = parts(&[
            ("host", "app.test"),
            ("referer", "http://app.test/posts?page=2"),
        ]);
        assert_eq!(
            redirect::back(&same, "/").headers()[LOCATION],
            "/posts?page=2"
        );
        let other = parts(&[("host", "app.test"), ("referer", "http://evil.test/x")]);
        assert_eq!(redirect::back(&other, "/home").headers()[LOCATION], "/home");
        assert_eq!(
            redirect::back(&parts(&[]), "/home").headers()[LOCATION],
            "/home"
        );
    }

    #[test]
    fn builders_chain_status_headers_and_cookies() {
        let response = json(serde_json::json!({ "ok": true }))
            .with_status(StatusCode::CREATED)
            .with_headers([(HeaderName::from_static("x-a"), "1")])
            .with_cookie(Cookie::new("seen", "a b").http_only().secure().max_age(60))
            .with_cookie(Cookie::forget("old"));
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()["x-a"], "1");
        let cookies: Vec<_> = response.headers().get_all(SET_COOKIE).iter().collect();
        assert_eq!(
            cookies[0],
            "seen=a%20b; Path=/; SameSite=Lax; Max-Age=60; HttpOnly; Secure"
        );
        assert!(cookies[1].to_str().unwrap().contains("Max-Age=0"));
        assert_eq!(no_content().status(), StatusCode::NO_CONTENT);
    }

    #[test]
    fn cookies_resist_attribute_injection() {
        let value = Cookie::new("a b", "v")
            .path("/; Domain=evil")
            .same_site(SameSite::None)
            .render();
        assert_eq!(value, "a%20b=v; Path=/ Domain=evil; SameSite=None; Secure");
        assert_eq!(redirect::away("bad\nurl").headers()[LOCATION], "/");
    }

    #[tokio::test]
    async fn files_download_and_stream() {
        let dir = std::env::temp_dir().join(format!("oxide-resp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("r.csv");
        std::fs::write(&path, "a,b\n").unwrap();
        let inline = file(&path).await.unwrap();
        assert_eq!(inline.headers()[CONTENT_TYPE], "text/csv; charset=utf-8");
        let attached = download(&path, "na\"me é.csv").await.unwrap();
        let disposition = attached.headers()[CONTENT_DISPOSITION].to_str().unwrap();
        assert!(
            disposition.starts_with("attachment; filename=\"na_me _.csv\""),
            "{disposition}"
        );
        assert_eq!(text(attached).await, "a,b\n");
        assert!(matches!(
            file(dir.join("missing")).await,
            Err(OxideHttpError::NotFound(_))
        ));
        let chunks = futures_core_stream();
        assert_eq!(text(stream(chunks)).await, "ab");
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn futures_core_stream() -> impl TryStream<Ok = Bytes, Error = axum::Error> + Send {
        axum::body::Body::from("ab").into_data_stream()
    }

    struct Post;
    impl Responsable for Post {
        fn to_response(self, request: &Parts) -> Response {
            if request.headers.contains_key("x-json") {
                json(serde_json::json!({ "post": 1 }))
            } else {
                redirect::to("/posts/1")
            }
        }
    }

    #[test]
    fn responsable_types_render_per_request() {
        assert_eq!(
            respond(Post, &parts(&[("x-json", "1")])).status(),
            StatusCode::OK
        );
        assert_eq!(respond(Post, &parts(&[])).status(), StatusCode::FOUND);
    }
}
