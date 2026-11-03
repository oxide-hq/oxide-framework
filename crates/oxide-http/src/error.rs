//! The unified, Laravel-style HTTP error contract.

use std::collections::BTreeMap;

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
}

impl OxideHttpError {
    /// The HTTP status this error maps to.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for OxideHttpError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = match self {
            Self::Validation(errors) => json!({
                "message": VALIDATION_MESSAGE,
                "errors": flatten_validation_errors(&errors),
            }),
            Self::Forbidden(message) if message.is_empty() => {
                json!({ "message": UNAUTHORIZED_MESSAGE })
            }
            Self::NotFound(message)
            | Self::Forbidden(message)
            | Self::BadRequest(message)
            | Self::Internal(message) => json!({ "message": message }),
        };
        (status, Json(body)).into_response()
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
