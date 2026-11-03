//! Middleware most applications want, ready to register with
//! [`Application::builder().middleware(..)`](crate::ApplicationBuilder::middleware) or
//! [`.alias(..)`](crate::ApplicationBuilder::alias).

mod cache;
mod cors;
mod hosts;
mod limit;
mod maintenance;
mod proxies;
mod security;
mod strings;

pub use cache::SetCacheHeaders;
pub use cors::{CorsConfig, HandleCors};
pub use hosts::TrustHosts;
pub use limit::ValidatePostSize;
pub use maintenance::{Down, PreventRequestsDuringMaintenance, down, is_down, up};
pub use proxies::{Client, ClientInfo, TrustProxies};
pub use security::SecurityHeaders;
pub use strings::{ConvertEmptyStringsToNull, TrimStrings};

use oxide_http::__private::axum::http::StatusCode;
use oxide_http::__private::axum::response::{IntoResponse, Response};
use oxide_http::{DomainError, OxideHttpError};

/// A rejection the framework's middleware makes: a status and a message for the client. It goes
/// through the exception handler like any other error, and is never reported as a failure.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct Rejection {
    status: StatusCode,
    message: String,
}

impl DomainError for Rejection {
    fn status(&self) -> StatusCode {
        self.status
    }

    fn message(&self) -> String {
        self.message.clone()
    }

    fn report(&self) -> bool {
        false
    }
}

fn reject(status: StatusCode, message: impl Into<String>) -> Response {
    OxideHttpError::domain(Rejection {
        status,
        message: message.into(),
    })
    .into_response()
}
