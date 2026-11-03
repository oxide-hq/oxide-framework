//! The unified, Laravel-style HTTP error contract.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use validator::{ValidationErrors, ValidationErrorsKind};

/// Message used for `403` responses when no custom message is given.
pub const UNAUTHORIZED_MESSAGE: &str = "This action is unauthorized.";

/// Message used for `422` validation responses (matches Laravel).
pub const VALIDATION_MESSAGE: &str = "The given data was invalid.";

/// Convenience alias for handler/extractor results.
pub type OxideResult<T> = Result<T, OxideHttpError>;

/// Every error Oxide's extractors produce. Converts into a JSON response shaped like Laravel's.
#[derive(Debug, thiserror::Error)]
pub enum OxideHttpError {
    /// `404 Not Found` — `{"message": "..."}`.
    #[error("not found: {0}")]
    NotFound(String),

    /// `403 Forbidden` — `{"message": "This action is unauthorized."}` unless a message is given.
    #[error("forbidden: {0}")]
    Forbidden(String),

    /// `400 Bad Request` — `{"message": "..."}`.
    #[error("bad request: {0}")]
    BadRequest(String),

    /// `422 Unprocessable Entity` — `{"message": "The given data was invalid.", "errors": {...}}`.
    #[error("validation failed: {0}")]
    Validation(#[from] ValidationErrors),

    /// `500 Internal Server Error` — `{"message": "..."}`.
    #[error("internal error: {0}")]
    Internal(String),

    /// `500` whose real cause is kept for reporting and debug pages but never sent to the
    /// client, which only sees `message` (use `"Server Error"`).
    #[error("{message}")]
    Server {
        message: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// An application error with its own status and client message: see [`DomainError`].
    #[error(transparent)]
    Domain(Arc<dyn DomainError>),
}

/// An application error that knows how it appears to clients, so `?` in a handler can turn it
/// into the right response.
///
/// ```ignore
/// #[derive(Debug, thiserror::Error)]
/// #[error("card declined: {0}")]
/// struct PaymentFailed(String);
///
/// impl DomainError for PaymentFailed {
///     fn status(&self) -> StatusCode { StatusCode::PAYMENT_REQUIRED }
///     fn message(&self) -> String { "Your card was declined.".into() }
/// }
/// impl From<PaymentFailed> for OxideHttpError {
///     fn from(error: PaymentFailed) -> Self { OxideHttpError::domain(error) }
/// }
/// ```
pub trait DomainError: std::error::Error + Send + Sync + 'static {
    /// The response status. Defaults to `500`.
    fn status(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    /// What the client is told. Defaults to the error's `Display`, so override it when that
    /// text is for developers.
    fn message(&self) -> String {
        self.to_string()
    }

    /// Whether the exception handler reports (logs) this error. Defaults to server errors only.
    fn report(&self) -> bool {
        self.status().is_server_error()
    }
}

/// Attached to every error response, so the exception handler can see which error produced it.
#[derive(Debug, Clone)]
pub struct HandledError(pub Arc<OxideHttpError>);

impl OxideHttpError {
    /// The HTTP status this error maps to.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Internal(_) | Self::Server { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Domain(error) => error.status(),
        }
    }

    /// An application error as a response; see [`DomainError`].
    pub fn domain(error: impl DomainError) -> Self {
        Self::Domain(Arc::new(error))
    }

    /// A `500 Server Error` that keeps `cause` for reporting without telling the client.
    pub fn server(cause: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Server {
            message: "Server Error".to_owned(),
            source: Box::new(cause),
        }
    }

    /// The JSON body clients receive: Laravel's `{"message": ..}` (plus `errors` for 422).
    pub fn body(&self) -> serde_json::Value {
        match self {
            Self::Validation(errors) => json!({
                "message": VALIDATION_MESSAGE,
                "errors": flatten_validation_errors(errors),
            }),
            Self::Forbidden(message) if message.is_empty() => {
                json!({ "message": UNAUTHORIZED_MESSAGE })
            }
            Self::NotFound(message)
            | Self::Forbidden(message)
            | Self::BadRequest(message)
            | Self::Internal(message)
            | Self::Server { message, .. } => json!({ "message": message }),
            Self::Domain(error) => json!({ "message": error.message() }),
        }
    }
}

impl IntoResponse for OxideHttpError {
    fn into_response(self) -> Response {
        let mut response = (self.status(), Json(self.body())).into_response();
        response
            .extensions_mut()
            .insert(HandledError(Arc::new(self)));
        response
    }
}

/// Flattens `validator` errors into Laravel's `{ "field": ["message", ...] }` map.
///
/// Nested structs use dot notation (`address.city`) and lists use indices (`items.0.name`).
pub fn flatten_validation_errors(errors: &ValidationErrors) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    collect(errors, None, &mut out);
    out
}

fn collect(
    errors: &ValidationErrors,
    prefix: Option<&str>,
    out: &mut BTreeMap<String, Vec<String>>,
) {
    for (field, kind) in errors.errors() {
        let path = match prefix {
            Some(prefix) => format!("{prefix}.{field}"),
            None => field.to_string(),
        };
        match kind {
            ValidationErrorsKind::Field(field_errors) => {
                let messages = out.entry(path.clone()).or_default();
                messages.extend(field_errors.iter().map(|error| match &error.message {
                    Some(message) => message.to_string(),
                    None => format!("The {path} field is invalid ({}).", error.code),
                }));
            }
            ValidationErrorsKind::Struct(nested) => collect(nested, Some(&path), out),
            ValidationErrorsKind::List(items) => {
                for (index, nested) in items {
                    collect(nested, Some(&format!("{path}.{index}")), out);
                }
            }
        }
    }
}
