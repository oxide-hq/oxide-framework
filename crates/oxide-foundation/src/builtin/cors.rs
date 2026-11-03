use oxide_config::Config;
use oxide_http::__private::axum::body::Body;
use oxide_http::__private::axum::extract::Request;
use oxide_http::__private::axum::http::{HeaderValue, Method, StatusCode, header};
use oxide_http::__private::axum::response::Response;
use serde::Deserialize;

use crate::middleware::{Middleware, MiddlewareFuture, MwNext};

/// `config/cors.toml`: which other origins may call the API from a browser.
///
/// ```toml
/// allowed_origins = ["https://app.example.com"]
/// supports_credentials = true
/// ```
#[derive(Clone, Deserialize, Config)]
#[config(file = "cors")]
pub struct CorsConfig {
    /// Exact origins (`https://app.example.com`), or `*` for any.
    #[config(default = ["*"])]
    pub allowed_origins: Vec<String>,
    #[config(default = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"])]
    pub allowed_methods: Vec<String>,
    /// Request headers browsers may send, or `*` for whichever they ask for.
    #[config(default = ["*"])]
    pub allowed_headers: Vec<String>,
    /// Response headers scripts may read, beyond the always-visible ones.
    #[serde(default)]
    pub exposed_headers: Vec<String>,
    /// Seconds a browser may cache a preflight answer.
    #[config(default = 0)]
    pub max_age: u64,
    /// Allow cookies and `Authorization` on cross-origin calls. Needs explicit origins.
    #[config(default = false)]
    pub supports_credentials: bool,
}

/// Cross-origin resource sharing: answers preflight requests and adds the `Access-Control-*`
/// headers to responses for allowed origins.
///
/// Requests without an `Origin` header are not cross-origin and pass through untouched.
#[derive(Debug, Clone)]
pub struct HandleCors {
    any_origin: bool,
    origins: Vec<String>,
    methods: Option<HeaderValue>,
    any_header: bool,
    headers: Option<HeaderValue>,
    exposed: Option<HeaderValue>,
    max_age: u64,
    credentials: bool,
}

impl HandleCors {
    /// Builds the middleware, refusing `supports_credentials` together with a wildcard origin
    /// (which would let any site make credentialed calls).
    pub fn new(config: &CorsConfig) -> Result<Self, String> {
        let any_origin = config.allowed_origins.iter().any(|origin| origin == "*");
        if any_origin && config.supports_credentials {
            return Err("cors: `supports_credentials` can't be combined with the `*` origin; list the allowed origins".to_owned());
        }
        let join = |values: &[String]| {
            (!values.is_empty())
                .then(|| HeaderValue::from_str(&values.join(", ")))
                .transpose()
                .map_err(|error| format!("cors: invalid header value: {error}"))
        };
        Ok(Self {
            any_origin,
            origins: config.allowed_origins.clone(),
            methods: join(&config.allowed_methods)?,
            any_header: config.allowed_headers.iter().any(|header| header == "*"),
            headers: join(&config.allowed_headers)?,
            exposed: join(&config.exposed_headers)?,
            max_age: config.max_age,
            credentials: config.supports_credentials,
        })
    }

    fn allows(&self, origin: &str) -> bool {
        self.any_origin || self.origins.iter().any(|allowed| allowed == origin)
    }

    fn allow_origin(&self, response: &mut Response, origin: &str) {
        let value = if self.any_origin {
            HeaderValue::from_static("*")
        } else if let Ok(value) = HeaderValue::from_str(origin) {
            value
        } else {
            return;
        };
        let headers = response.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
        if self.credentials {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
                HeaderValue::from_static("true"),
            );
        }
    }
}

fn append_vary(response: &mut Response, names: &str) {
    if let Ok(value) = HeaderValue::from_str(names) {
        response.headers_mut().append(header::VARY, value);
    }
}

impl Middleware for HandleCors {
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        _params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(async move {
            let Some(origin) = request
                .headers()
                .get(header::ORIGIN)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
            else {
                // Not a CORS request, but with specific origins the answer still depends on
                // `Origin`, so caches must be told not to reuse it for cross-origin requests.
                let mut response = next.run(request).await;
                if !self.any_origin {
                    append_vary(&mut response, "Origin");
                }
                return response;
            };
            let allowed = self.allows(&origin);

            let is_preflight = request.method() == Method::OPTIONS
                && request
                    .headers()
                    .contains_key(header::ACCESS_CONTROL_REQUEST_METHOD);
            if is_preflight {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = StatusCode::NO_CONTENT;
                if allowed {
                    self.allow_origin(&mut response, &origin);
                    if let Some(methods) = &self.methods {
                        response
                            .headers_mut()
                            .insert(header::ACCESS_CONTROL_ALLOW_METHODS, methods.clone());
                    }
                    let requested = request
                        .headers()
                        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
                        .cloned();
                    let allow_headers = if self.any_header {
                        requested
                    } else {
                        self.headers.clone()
                    };
                    if let Some(value) = allow_headers {
                        response
                            .headers_mut()
                            .insert(header::ACCESS_CONTROL_ALLOW_HEADERS, value);
                    }
                    if self.max_age > 0 {
                        response.headers_mut().insert(
                            header::ACCESS_CONTROL_MAX_AGE,
                            HeaderValue::from(self.max_age),
                        );
                    }
                }
                append_vary(
                    &mut response,
                    "Origin, Access-Control-Request-Method, Access-Control-Request-Headers",
                );
                return response;
            }

            let mut response = next.run(request).await;
            if allowed {
                self.allow_origin(&mut response, &origin);
                if let Some(exposed) = &self.exposed {
                    response
                        .headers_mut()
                        .insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, exposed.clone());
                }
            }
            append_vary(&mut response, "Origin");
            response
        })
    }
}
