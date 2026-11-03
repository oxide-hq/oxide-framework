use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use oxide_http::__private::axum::extract::Request;
use oxide_http::__private::axum::http::{HeaderValue, StatusCode, header};
use serde::{Deserialize, Serialize};

use super::reject;
use crate::middleware::{Middleware, MiddlewareFuture, MwNext};

/// How long the "is the app down?" answer is reused, so a busy app doesn't read the file on
/// every request.
const CHECK_EVERY: Duration = Duration::from_secs(1);

/// What `down` writes: how maintenance mode behaves.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Down {
    /// Seconds clients should wait before retrying (`Retry-After`).
    #[serde(default)]
    pub retry: Option<u64>,
    /// What clients are told. Defaults to "Service Unavailable".
    #[serde(default)]
    pub message: Option<String>,
    /// Requests sending this value in `X-Maintenance-Secret` get through, for testing a
    /// deploy before reopening.
    #[serde(default)]
    pub secret: Option<String>,
}

/// Puts the app into maintenance mode by writing `file` (the middleware notices within a
/// second).
pub fn down(file: &Path, settings: &Down) -> io::Result<()> {
    if let Some(parent) = file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(io::Error::other)?;
    std::fs::write(file, json)
}

/// Takes the app out of maintenance mode. Returns whether it was down.
pub fn up(file: &Path) -> io::Result<bool> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub fn is_down(file: &Path) -> bool {
    file.is_file()
}

/// Answers `503 Service Unavailable` while the maintenance file exists (see [`down`] and
/// [`up`]), except for the paths you allow and requests carrying the secret.
///
/// A path ending in `*` matches by prefix: `"/health*"`.
#[derive(Debug)]
pub struct PreventRequestsDuringMaintenance {
    file: PathBuf,
    except: Vec<String>,
    cache: Mutex<Option<(Instant, Option<Down>)>>,
}

impl PreventRequestsDuringMaintenance {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self {
            file: file.into(),
            except: Vec::new(),
            cache: Mutex::new(None),
        }
    }

    /// Paths that stay up during maintenance (health checks, say).
    #[must_use]
    pub fn except<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.except.extend(paths.into_iter().map(Into::into));
        self
    }

    /// The current settings if the app is down, from the file at most once a second.
    fn current(&self) -> Option<Down> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((checked, state)) = cache.as_ref()
            && checked.elapsed() < CHECK_EVERY
        {
            return state.clone();
        }
        let state = std::fs::read_to_string(&self.file).ok().map(|text| {
            // A file that isn't valid settings still means "down", with the defaults.
            serde_json::from_str::<Down>(&text).unwrap_or_default()
        });
        *cache = Some((Instant::now(), state.clone()));
        state
    }

    fn excepted(&self, path: &str) -> bool {
        self.except
            .iter()
            .any(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => path.starts_with(prefix),
                None => pattern == path,
            })
    }
}

/// Compares secrets without stopping at the first difference.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0_u8, |diff, (x, y)| diff | (x ^ y))
            == 0
}

impl Middleware for PreventRequestsDuringMaintenance {
    fn handle<'a>(
        &'a self,
        request: Request,
        next: MwNext,
        _params: &'a [String],
    ) -> MiddlewareFuture<'a> {
        Box::pin(async move {
            let Some(settings) = self.current() else {
                return next.run(request).await;
            };
            let has_secret = settings.secret.as_deref().is_some_and(|secret| {
                request
                    .headers()
                    .get("x-maintenance-secret")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|sent| same_secret(sent, secret))
            });
            if has_secret || self.excepted(request.uri().path()) {
                return next.run(request).await;
            }
            let mut response = reject(
                StatusCode::SERVICE_UNAVAILABLE,
                settings
                    .message
                    .unwrap_or_else(|| "Service Unavailable".to_owned()),
            );
            if let Some(retry) = settings.retry {
                response
                    .headers_mut()
                    .insert(header::RETRY_AFTER, HeaderValue::from(retry));
            }
            response
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_match_exactly_or_by_prefix() {
        let middleware = PreventRequestsDuringMaintenance::new("x").except(["/up", "/health*"]);
        assert!(middleware.excepted("/up"));
        assert!(middleware.excepted("/health/db"));
        assert!(!middleware.excepted("/upgrade"));
        assert!(!middleware.excepted("/"));
    }

    #[test]
    fn secrets_compare_fully() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("abc", "abcd"));
        assert!(!same_secret("", "a"));
    }
}
