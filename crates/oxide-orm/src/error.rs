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

    /// A migration failed to apply.
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

impl OrmError {
    /// A [`OrmError::NotFound`] for `model`, optionally naming the key that was looked up.
    pub fn not_found(model: &'static str, id: Option<String>) -> Self {
        Self::NotFound { model, id }
    }
}

/// `NotFound` becomes a `404`; database and migration failures become a `500` without leaking
/// SQL details.
impl From<OrmError> for OxideHttpError {
    fn from(error: OrmError) -> Self {
        match error {
            OrmError::NotFound { .. } => Self::NotFound(error.to_string()),
            // The client sees "Server Error"; the real cause stays attached for the exception
            // handler to report.
            OrmError::Database(_) | OrmError::Migrate(_) => Self::server(error),
        }
    }
}

impl IntoResponse for OrmError {
    fn into_response(self) -> Response {
        OxideHttpError::from(self).into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn database_failures_keep_their_cause_but_tell_the_client_nothing() {
        let error = OxideHttpError::from(OrmError::Database(sqlx::Error::PoolClosed));
        assert_eq!(
            error.body(),
            serde_json::json!({ "message": "Server Error" })
        );
        assert!(matches!(error, OxideHttpError::Server { .. }));
        let cause = error.source().expect("the cause is attached");
        assert!(cause.to_string().contains("pool"), "{cause}");
    }
}
