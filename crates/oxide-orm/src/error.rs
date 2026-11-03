//! ORM errors and their mapping onto the HTTP error contract.

use axum::response::{IntoResponse, Response};
use oxide_http::OxideHttpError;

/// Errors returned by models and queries.
#[derive(Debug, thiserror::Error)]
pub enum OrmError {
    /// No row matched (`find_or_fail`, `first_or_fail`, `save` on a deleted row, ...).
    #[error("No query results for model [{model}]{}", id.as_ref().map(|id| format!(" {id}")).unwrap_or_default())]
    NotFound {
        /// The model's type name, e.g. `"Post"`.
        model: &'static str,
        /// The primary key that was looked up, when known.
        id: Option<String>,
    },

    /// The database reported an error.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl OrmError {
    /// A [`OrmError::NotFound`] for `model`, optionally naming the key that was looked up.
    pub fn not_found(model: &'static str, id: Option<String>) -> Self {
        Self::NotFound { model, id }
    }
}

/// `NotFound` becomes a `404`; database failures become a `500` without leaking SQL details.
impl From<OrmError> for OxideHttpError {
    fn from(error: OrmError) -> Self {
        match error {
            OrmError::NotFound { .. } => Self::NotFound(error.to_string()),
            OrmError::Database(_) => Self::Internal("Server Error".into()),
        }
    }
}

impl IntoResponse for OrmError {
    fn into_response(self) -> Response {
        OxideHttpError::from(self).into_response()
    }
}
