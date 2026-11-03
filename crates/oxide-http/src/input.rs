//! [`Input`]: the request's query string and body as one value, like Laravel's
//! `$request->input('user.name')`.
//!
//! The body (JSON, or a form with PHP-style keys such as `user[name]` and `tags[]`) takes
//! precedence over the query string. Use it when you want to *look at* input: for typed,
//! validated payloads prefer [`Validated<T>`](crate::Validated). `Input` reads the whole body, so
//! take it in place of, not alongside, another body extractor.

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use oxide_support::Arr;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::error::OxideHttpError;
use crate::request_ext::media_type;

/// Query string and body, merged. See the [module docs](self).
#[derive(Debug, Clone, Default)]
pub struct Input {
    query: Value,
    body: Value,
}

impl Input {
    /// Builds input from a raw query string, a `Content-Type` and a body (for tests and
    /// middleware that already hold the bytes).
    pub fn parse(
        query: Option<&str>,
        content_type: Option<&str>,
        body: &[u8],
    ) -> Result<Self, OxideHttpError> {
        let query = query.map_or_else(empty_object, form_value);
        let media = content_type.map(media_type).unwrap_or_default();
        let body = if body.is_empty() {
            empty_object()
        } else if media == "application/json" || media.ends_with("+json") {
            serde_json::from_slice::<Value>(body)
                .map_err(|error| OxideHttpError::BadRequest(format!("Malformed JSON: {error}")))?
        } else if media == "application/x-www-form-urlencoded" {
            form_value(&String::from_utf8_lossy(body))
        } else {
            empty_object()
        };
        Ok(Self { query, body })
    }

    /// Everything, as one JSON value: the query string's keys with the body's keys on top.
    pub fn all(&self) -> Value {
        let mut merged = match &self.query {
            Value::Object(map) => map.clone(),
            _ => Map::new(),
        };
        match &self.body {
            Value::Object(map) => merged.extend(map.clone()),
            Value::Null => {}
            other => return other.clone(),
        }
        Value::Object(merged)
    }

    /// A value by dot path (`user.name`, `items.0.id`): from the body, else the query string.
    pub fn input(&self, key: &str) -> Option<&Value> {
        Arr::get(&self.body, key).or_else(|| Arr::get(&self.query, key))
    }

    /// A value from the query string only.
    pub fn query(&self, key: &str) -> Option<&Value> {
        Arr::get(&self.query, key)
    }

    /// Whether the key is present, even if empty or `null`.
    pub fn has(&self, key: &str) -> bool {
        self.input(key).is_some()
    }

    /// Whether every key is present.
    pub fn has_all(&self, keys: &[&str]) -> bool {
        keys.iter().all(|key| self.has(key))
    }

    /// Whether any key is present.
    pub fn has_any(&self, keys: &[&str]) -> bool {
        keys.iter().any(|key| self.has(key))
    }

    /// Whether the key is present with something in it: not `null`, and not an empty or blank
    /// string.
    pub fn filled(&self, key: &str) -> bool {
        self.input(key).is_some_and(|value| match value {
            Value::Null => false,
            Value::String(text) => !text.trim().is_empty(),
            _ => true,
        })
    }

    pub fn missing(&self, key: &str) -> bool {
        !self.has(key)
    }

    /// A string; numbers and booleans are written out, `null`, arrays and objects give `None`.
    pub fn string(&self, key: &str) -> Option<String> {
        match self.input(key)? {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        }
    }

    /// An integer, from a number or a numeric string (`"42"`).
    pub fn integer(&self, key: &str) -> Option<i64> {
        match self.input(key)? {
            Value::Number(number) => number.as_i64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        }
    }

    /// A float, from a number or a numeric string.
    pub fn float(&self, key: &str) -> Option<f64> {
        match self.input(key)? {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        }
    }

    /// A boolean the way forms write them: `true`, `1`, `"on"`, `"yes"` are true; `false`, `0`,
    /// `"off"`, `"no"` are false. `None` for anything else or a missing key.
    pub fn try_boolean(&self, key: &str) -> Option<bool> {
        match self.input(key)? {
            Value::Bool(flag) => Some(*flag),
            Value::Number(number) => match number.as_i64() {
                Some(1) => Some(true),
                Some(0) => Some(false),
                _ => None,
            },
            Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "on" | "yes" => Some(true),
                "0" | "false" | "off" | "no" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    /// Like [`try_boolean`](Self::try_boolean), but anything unrecognised or missing is `false`,
    /// as Laravel's `boolean()`.
    pub fn boolean(&self, key: &str) -> bool {
        self.try_boolean(key).unwrap_or(false)
    }

    /// A date-time (UTC) from RFC 3339 (`2026-11-03T09:30:00Z`), `YYYY-MM-DD HH:MM:SS` or a bare
    /// `YYYY-MM-DD` (midnight).
    pub fn date(&self, key: &str) -> Option<DateTime<Utc>> {
        let Value::String(text) = self.input(key)? else {
            return None;
        };
        let text = text.trim();
        if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
            return Some(parsed.with_timezone(&Utc));
        }
        if let Ok(parsed) = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S") {
            return Some(parsed.and_utc());
        }
        NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .ok()?
            .and_hms_opt(0, 0, 0)
            .map(|midnight| midnight.and_utc())
    }

    /// A value deserialized into any type, such as an enum with `#[serde(rename_all = "..")]`:
    /// `input.get::<Status>("status")`.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        serde_json::from_value(self.input(key)?.clone()).ok()
    }

    /// An object holding only these top-level keys.
    pub fn only(&self, keys: &[&str]) -> Value {
        Arr::only(&self.all(), keys)
    }

    /// An object without these top-level keys.
    pub fn except(&self, keys: &[&str]) -> Value {
        Arr::except(&self.all(), keys)
    }
}

impl<S: Send + Sync> FromRequest<S> for Input {
    type Rejection = OxideHttpError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let query = request.uri().query().map(str::to_owned);
        let content_type = request
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = Bytes::from_request(request, state)
            .await
            .map_err(|rejection| {
                if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    OxideHttpError::with_status(StatusCode::PAYLOAD_TOO_LARGE, "Payload Too Large")
                } else {
                    OxideHttpError::BadRequest("The request body could not be read.".to_owned())
                }
            })?;
        Self::parse(query.as_deref(), content_type.as_deref(), &bytes)
    }
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

/// A form or query string as JSON. Keys may use PHP's brackets: `user[name]=x` nests, and
/// `tags[]=a&tags[]=b` builds a list.
fn form_value(encoded: &str) -> Value {
    let mut root = empty_object();
    let pairs = serde_urlencoded::from_str::<Vec<(String, String)>>(encoded).unwrap_or_default();
    for (key, value) in pairs {
        let append = key.ends_with("[]");
        let key = key.strip_suffix("[]").unwrap_or(&key);
        let path = key.replace("][", ".").replace(['[', ']'], ".");
        let path = path.trim_matches('.');
        if path.is_empty() {
            continue;
        }
        let value = Value::String(value);
        if append {
            let mut list = match Arr::get(&root, path) {
                Some(Value::Array(items)) => items.clone(),
                _ => Vec::new(),
            };
            list.push(value);
            Arr::set(&mut root, path, Value::Array(list));
        } else {
            Arr::set(&mut root, path, value);
        }
    }
    root
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    fn form(body: &str) -> Input {
        Input::parse(
            None,
            Some("application/x-www-form-urlencoded"),
            body.as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn php_style_form_keys_nest_and_collect() {
        let input =
            form("user[name]=Ann&user[address][city]=Lisbon&tags[]=a&tags[]=b&plain=1&sp=a%20b+c");
        assert_eq!(
            input.all(),
            json!({
                "user": { "name": "Ann", "address": { "city": "Lisbon" } },
                "tags": ["a", "b"],
                "plain": "1",
                "sp": "a b c",
            })
        );
        assert_eq!(input.string("user.address.city").as_deref(), Some("Lisbon"));
        assert_eq!(input.input("tags.1"), Some(&json!("b")));
    }

    #[test]
    fn the_body_beats_the_query_string() {
        let input = Input::parse(
            Some("page=2&sort=name"),
            Some("application/json"),
            br#"{"sort":"date"}"#,
        )
        .unwrap();
        assert_eq!(input.string("sort").as_deref(), Some("date"));
        assert_eq!(input.integer("page"), Some(2));
        assert_eq!(input.query("sort"), Some(&json!("name")));
        assert_eq!(input.all(), json!({ "page": "2", "sort": "date" }));
    }

    #[test]
    fn typed_getters_coerce_the_way_forms_need() {
        let input = Input::parse(
            None,
            Some("application/json"),
            br#"{"n":"42","f":"1.5","t":"on","no":"off","one":1,"odd":"maybe","d":"2026-11-03","dt":"2026-11-03T09:30:00+01:00","s":7,"list":[1],"nul":null,"blank":"  "}"#,
        )
        .unwrap();
        assert_eq!(
            (input.integer("n"), input.integer("s"), input.integer("f")),
            (Some(42), Some(7), None)
        );
        assert_eq!(input.float("f"), Some(1.5));
        assert_eq!(
            (
                input.try_boolean("t"),
                input.try_boolean("no"),
                input.try_boolean("one")
            ),
            (Some(true), Some(false), Some(true))
        );
        assert_eq!(input.try_boolean("odd"), None);
        assert!(input.boolean("t") && !input.boolean("odd") && !input.boolean("missing"));
        assert_eq!(
            input.date("d").unwrap().to_rfc3339(),
            "2026-11-03T00:00:00+00:00"
        );
        assert_eq!(
            input.date("dt").unwrap().to_rfc3339(),
            "2026-11-03T08:30:00+00:00"
        );
        assert_eq!(input.date("odd"), None);
        assert_eq!(input.string("s").as_deref(), Some("7"));
        assert_eq!(input.string("list"), None);

        assert!(
            input.has("nul") && !input.filled("nul") && !input.filled("blank") && input.filled("s")
        );
        assert!(input.missing("nope") && input.has_all(&["n", "f"]) && input.has_any(&["x", "n"]));
        assert!(!input.has_all(&["n", "x"]));
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "snake_case")]
    enum Status {
        Draft,
        Published,
    }

    #[test]
    fn values_deserialize_into_types_and_subsets_are_cut() {
        let input = Input::parse(
            None,
            Some("application/json"),
            br#"{"status":"published","a":1,"b":2}"#,
        )
        .unwrap();
        assert_eq!(input.get::<Status>("status"), Some(Status::Published));
        assert_eq!(input.get::<Status>("a"), None);
        assert_eq!(input.get::<Vec<u8>>("missing"), None);
        assert_eq!(input.only(&["a"]), json!({ "a": 1 }));
        assert_eq!(input.except(&["a", "status"]), json!({ "b": 2 }));
        assert_eq!(Status::Draft, Status::Draft);
    }

    #[test]
    fn bodies_that_are_not_understood_are_empty_or_errors() {
        assert_eq!(
            Input::parse(None, Some("text/plain"), b"hello")
                .unwrap()
                .all(),
            json!({})
        );
        assert_eq!(Input::parse(None, None, b"").unwrap().all(), json!({}));
        let error = Input::parse(None, Some("application/json"), b"{ nope").unwrap_err();
        assert!(
            matches!(error, OxideHttpError::BadRequest(message) if message.starts_with("Malformed JSON"))
        );
        // A JSON array body is addressable by index.
        let list = Input::parse(None, Some("application/json"), br#"[{"id":1}]"#).unwrap();
        assert_eq!(list.input("0.id"), Some(&json!(1)));
        assert_eq!(list.all(), json!([{ "id": 1 }]));
    }
}
