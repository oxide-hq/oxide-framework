//! Laravel-style form requests: deserialize, authorize, then validate.

use std::future::Future;
use std::ops::{Deref, DerefMut};

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use serde::de::DeserializeOwned;
use validator::ValidationErrors;

use crate::error::{OxideHttpError, UNAUTHORIZED_MESSAGE};

/// A request payload that knows how to authorize and validate itself.
///
/// Usually implemented with `#[derive(Deserialize, Validate, FormRequest)]`.
pub trait FormRequest<S>: Sized + Send + DeserializeOwned {
    /// Whether the current request may perform this action. Defaults to `true`.
    fn authorize(&self, _parts: &Parts, _state: &S) -> impl Future<Output = bool> + Send {
        async { true }
    }

    /// Validates the payload's fields.
    fn validate_fields(&self) -> Result<(), ValidationErrors>;
}

/// Custom authorization for a derived [`FormRequest`].
///
/// Enable it with `#[form_request(authorize)]`; implement it for the concrete state type(s) your
/// routes use.
pub trait Authorize<S> {
    /// Whether the current request may perform this action.
    fn authorize(&self, parts: &Parts, state: &S) -> impl Future<Output = bool> + Send;
}

/// Extractor that deserializes (JSON or form-urlencoded), authorizes and validates `T`.
///
/// Rejects with `400` on malformed input, `403` when unauthorized and `422` when invalid.
#[derive(Debug, Clone, Copy, Default)]
pub struct Validated<T>(pub T);

impl<T> Deref for Validated<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Validated<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<S, T> FromRequest<S> for Validated<T>
where
    S: Send + Sync,
    T: FormRequest<S>,
{
    type Rejection = OxideHttpError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (parts, body) = request.into_parts();
        let content_type = parts
            .headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
            })
            .unwrap_or_default();

        // `Bytes` honours `DefaultBodyLimit`, so we keep axum's body-size protection.
        let bytes = Bytes::from_request(Request::from_parts(parts.clone(), body), state)
            .await
            .map_err(|rejection| OxideHttpError::BadRequest(rejection.body_text()))?;

        let payload: T = match content_type.as_str() {
            "application/json" => parse_json(&bytes)?,
            ct if ct.starts_with("application/") && ct.ends_with("+json") => parse_json(&bytes)?,
            "application/x-www-form-urlencoded" => serde_urlencoded::from_bytes(&bytes)
                .map_err(|e| OxideHttpError::BadRequest(format!("Malformed form payload: {e}")))?,
            "" => {
                return Err(OxideHttpError::BadRequest(
                    "Missing Content-Type header; expected application/json or application/x-www-form-urlencoded."
                        .into(),
                ));
            }
            other => {
                return Err(OxideHttpError::BadRequest(format!(
                    "Unsupported Content-Type `{other}`; expected application/json or application/x-www-form-urlencoded."
                )));
            }
        };

        if !payload.authorize(&parts, state).await {
            return Err(OxideHttpError::Forbidden(UNAUTHORIZED_MESSAGE.into()));
        }

        payload.validate_fields()?;

        Ok(Self(payload))
    }
}

fn parse_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, OxideHttpError> {
    serde_json::from_slice(bytes)
        .map_err(|e| OxideHttpError::BadRequest(format!("Malformed JSON payload: {e}")))
}
