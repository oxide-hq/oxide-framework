//! The exception handler: one place where errors are reported and rendered.
//!
//! Every error response that comes from [`OxideHttpError`] carries the error that produced it
//! (see [`HandledError`]). A middleware installed by [`Application`](crate::Application) finds
//! it after the handler ran, asks the [`ExceptionHandler`] whether to report it, lets the
//! handler render the final response, and otherwise changes nothing.

use std::error::Error as _;
use std::fmt::Write as _;
use std::sync::Arc;

use oxide_http::__private::axum::Json;
use oxide_http::__private::axum::extract::{Request, State};
use oxide_http::__private::axum::http::{HeaderValue, StatusCode, header};
use oxide_http::__private::axum::middleware::Next;
use oxide_http::__private::axum::response::{IntoResponse, Response};
use oxide_http::error::flatten_validation_errors;
use oxide_http::{HandledError, OxideHttpError, negotiate};
use oxide_log::tracing;
use serde_json::{Value, json};

/// What an error handler needs to know about the request that failed.
#[derive(Debug, Clone)]
pub struct RequestInfo {
    pub method: String,
    pub path: String,
    /// Whether the client prefers an HTML page (a browser) over JSON (an API client).
    pub wants_html: bool,
}

/// Whether the client prefers HTML to JSON, by `q` value and then by order: browsers send
/// `text/html` first, API clients send `application/json` or `*/*`.
fn prefers_html(accept: Option<&str>) -> bool {
    negotiate(accept, &["application/json", "text/html"]) == Some("text/html")
}

/// The request and the application's debug setting, passed to every hook.
#[derive(Debug, Clone, Copy)]
pub struct ErrorContext<'a> {
    pub request: &'a RequestInfo,
    /// True only when debug output is on *and* the environment is `local`: render the exception
    /// chain. Never true in production.
    pub debug: bool,
}

/// Decides which errors are reported and how every error response looks. Override a hook to
/// change it; the `default_*` functions give you the standard behaviour to delegate to.
pub trait ExceptionHandler: Send + Sync + 'static {
    /// Whether to report (log) `error`. Defaults to server errors, or a [`DomainError`]'s own
    /// answer.
    ///
    /// [`DomainError`]: oxide_http::DomainError
    fn should_report(&self, error: &OxideHttpError) -> bool {
        default_should_report(error)
    }

    /// Reports `error`. Defaults to a `tracing` event carrying the cause chain, the request, and
    /// the request's `Context` (so the request id).
    fn report(&self, error: &OxideHttpError, context: &ErrorContext<'_>) {
        default_report(error, context);
    }

    /// Turns `response` (the one the error produced) into the final response. Defaults to an
    /// HTML page for browsers, and JSON with the exception chain in debug mode.
    fn render(
        &self,
        error: &OxideHttpError,
        context: &ErrorContext<'_>,
        response: Response,
    ) -> Response {
        default_render(error, context, response)
    }
}

/// The standard handler. Add statuses that should never be reported with
/// [`dont_report`](Self::dont_report).
#[derive(Debug, Clone, Default)]
pub struct DefaultExceptionHandler {
    dont_report: Vec<StatusCode>,
}

impl DefaultExceptionHandler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Never report errors with this status, even a server error.
    #[must_use]
    pub fn dont_report(mut self, status: StatusCode) -> Self {
        self.dont_report.push(status);
        self
    }
}

impl ExceptionHandler for DefaultExceptionHandler {
    fn should_report(&self, error: &OxideHttpError) -> bool {
        !self.dont_report.contains(&error.status()) && default_should_report(error)
    }
}

pub fn default_should_report(error: &OxideHttpError) -> bool {
    match error {
        OxideHttpError::Domain(domain) => domain.report(),
        other => other.status().is_server_error(),
    }
}

pub fn default_report(error: &OxideHttpError, context: &ErrorContext<'_>) {
    let status = error.status();
    let chain = exception_chain(error).join(" <- caused by: ");
    if status.is_server_error() {
        tracing::error!(
            status = status.as_u16(),
            method = %context.request.method,
            path = %context.request.path,
            error = %chain,
            "request failed"
        );
    } else {
        tracing::warn!(
            status = status.as_u16(),
            method = %context.request.method,
            path = %context.request.path,
            error = %chain,
            "request failed"
        );
    }
}

pub fn default_render(
    error: &OxideHttpError,
    context: &ErrorContext<'_>,
    response: Response,
) -> Response {
    if context.request.wants_html {
        let page = html_page(error, context.debug);
        return replace_body(response, page.into_response(), "text/html; charset=utf-8");
    }
    if context.debug {
        let mut body = error.body();
        if let Value::Object(map) = &mut body {
            let chain = exception_chain(error);
            map.insert(
                "exception".to_owned(),
                json!({ "message": chain.first(), "chain": chain }),
            );
        }
        return replace_body(response, Json(body).into_response(), "application/json");
    }
    response
}

/// The error and each of its causes, outermost first.
fn exception_chain(error: &OxideHttpError) -> Vec<String> {
    let mut chain = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        chain.push(cause.to_string());
        source = cause.source();
    }
    chain
}

/// `replacement`'s body and content type with `original`'s status and other headers.
fn replace_body(
    original: Response,
    mut replacement: Response,
    content_type: &'static str,
) -> Response {
    let (parts, _body) = original.into_parts();
    *replacement.status_mut() = parts.status;
    for (name, value) in &parts.headers {
        if name != header::CONTENT_LENGTH && name != header::CONTENT_TYPE {
            replacement
                .headers_mut()
                .insert(name.clone(), value.clone());
        }
    }
    replacement
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    replacement.extensions_mut().extend(parts.extensions);
    replacement
}

fn html_page(error: &OxideHttpError, debug: bool) -> String {
    let status = error.status();
    let reason = status.canonical_reason().unwrap_or("Error");
    let message = error.body()["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let fields = match error {
        OxideHttpError::Validation(errors) => {
            let mut items = String::new();
            for (field, messages) in flatten_validation_errors(errors) {
                let _ = write!(
                    items,
                    "<li><strong>{}</strong>: {}</li>",
                    escape(&field),
                    escape(&messages.join(" "))
                );
            }
            format!("<ul>{items}</ul>")
        }
        _ => String::new(),
    };
    let detail = if debug {
        format!(
            "<h2>Exception</h2><pre>{}</pre>",
            escape(&exception_chain(error).join("\n  caused by: "))
        )
    } else {
        String::new()
    };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{code} {reason}</title>\
         <style>body{{font-family:system-ui,sans-serif;margin:0;display:grid;place-items:center;min-height:100vh;color:#222}}\
         main{{max-width:40rem;padding:2rem}}h1{{font-size:4rem;margin:0}}.reason{{color:#666;margin:.25rem 0 1rem}}\
         pre{{background:#f4f4f4;padding:1rem;overflow:auto}}</style></head>\
         <body><main><h1>{code}</h1><p class=\"reason\">{reason}</p><p>{message}</p>{fields}{detail}</main></body></html>",
        code = status.as_u16(),
        reason = escape(reason),
        message = escape(&message),
    )
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[derive(Clone)]
pub(crate) struct ExceptionState {
    pub(crate) handler: Arc<dyn ExceptionHandler>,
    pub(crate) debug: bool,
}

pub(crate) async fn exception_middleware(
    State(state): State<ExceptionState>,
    request: Request,
    next: Next,
) -> Response {
    // Cheap clones now; the owned `RequestInfo` is built only if the response is an error.
    let method = request.method().clone();
    let uri = request.uri().clone();
    let accept = request.headers().get(header::ACCEPT).cloned();
    let response = next.run(request).await;
    let Some(HandledError(error)) = response.extensions().get::<HandledError>().cloned() else {
        return response;
    };
    let info = RequestInfo {
        method: method.to_string(),
        path: uri.path().to_owned(),
        // An unreadable header is treated as an empty one: nothing acceptable, so JSON.
        wants_html: prefers_html(
            accept
                .as_ref()
                .map(|value| value.to_str().unwrap_or_default()),
        ),
    };
    let context = ErrorContext {
        request: &info,
        debug: state.debug,
    };
    if state.handler.should_report(&error) {
        state.handler.report(&error, &context);
    }
    let mut response = state.handler.render(&error, &context, response);
    // The body depends on `Accept`, so caches must keep the variants apart.
    let varies_on_accept = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .any(|value| {
            value
                .to_str()
                .is_ok_and(|v| v.to_ascii_lowercase().contains("accept") || v.trim() == "*")
        });
    if !varies_on_accept {
        response
            .headers_mut()
            .append(header::VARY, HeaderValue::from_static("Accept"));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsers_get_html_and_api_clients_json() {
        assert!(prefers_html(Some(
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
        )));
        assert!(prefers_html(Some("text/html, application/json")));
        assert!(!prefers_html(Some("application/json, text/html")));
        assert!(!prefers_html(Some("*/*")));
        assert!(!prefers_html(Some("application/json")));
        assert!(!prefers_html(Some("application/vnd.api+json")));
        assert!(!prefers_html(Some("")));
        // Quality values decide before order does.
        assert!(!prefers_html(Some("text/html;q=0.1, application/json")));
        assert!(prefers_html(Some("application/json;q=0.5, text/html")));
        assert!(prefers_html(Some(
            "text/html;q=0.9, application/json;q=0.8"
        )));
        assert!(!prefers_html(Some("text/html;q=0, */*")));
        // Media types are case-insensitive; an unreadable q counts as 1.
        assert!(prefers_html(Some("TEXT/HTML;q=oops, application/json")));
    }

    #[test]
    fn html_escaping_covers_markup_and_quotes() {
        assert_eq!(
            escape(r#"<a href="x">'&'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
    }
}
