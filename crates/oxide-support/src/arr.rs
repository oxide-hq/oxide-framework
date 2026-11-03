//! Dot-notation access to `serde_json::Value`, like Laravel's `Arr::get('a.b.0.c')`.

use serde_json::{Map, Value};

/// `Arr` helpers over JSON values. A path is segments joined by `.`; a numeric segment indexes
/// an array.
#[derive(Debug, Clone, Copy)]
pub struct Arr;

impl Arr {
    /// The value at `path`, if every segment exists.
    pub fn get<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
        if path.is_empty() {
            return Some(value);
        }
        path.split('.')
            .try_fold(value, |current, segment| match current {
                Value::Object(map) => map.get(segment),
                Value::Array(items) => segment
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| items.get(index)),
                _ => None,
            })
    }

    pub fn has(value: &Value, path: &str) -> bool {
        Self::get(value, path).is_some()
    }

    /// Sets the value at `path`, creating objects along the way (and replacing scalars that are
    /// in the way). A numeric segment into an existing array sets or appends by index.
    pub fn set(value: &mut Value, path: &str, new: Value) {
        if path.is_empty() {
            *value = new;
            return;
        }
        let mut current = value;
        let mut segments = path.split('.').peekable();
        while let Some(segment) = segments.next() {
            let last = segments.peek().is_none();
            current = step_into(current, segment);
            if last {
                *current = new;
                return;
            }
        }
    }

    /// Removes and returns the value at `path`.
    pub fn forget(value: &mut Value, path: &str) -> Option<Value> {
        let (parent_path, key) = path.rsplit_once('.').unwrap_or(("", path));
        let parent = if parent_path.is_empty() {
            value
        } else {
            Self::get_mut(value, parent_path)?
        };
        match parent {
            Value::Object(map) => map.remove(key),
            Value::Array(items) => {
                let index = key
                    .parse::<usize>()
                    .ok()
                    .filter(|index| *index < items.len())?;
                Some(items.remove(index))
            }
            _ => None,
        }
    }

    fn get_mut<'a>(value: &'a mut Value, path: &str) -> Option<&'a mut Value> {
        path.split('.')
            .try_fold(value, |current, segment| match current {
                Value::Object(map) => map.get_mut(segment),
                Value::Array(items) => segment
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| items.get_mut(index)),
                _ => None,
            })
    }

    /// The value at `path` of every item of an array, skipping items without it.
    pub fn pluck(items: &Value, path: &str) -> Vec<Value> {
        items
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| Self::get(item, path).cloned())
            .collect()
    }

    /// Flattens nested objects and arrays into `a.b.0` keys. Empty containers stay as leaves.
    pub fn dot(value: &Value) -> Map<String, Value> {
        let mut out = Map::new();
        flatten(value, String::new(), &mut out);
        out
    }

    /// The inverse of [`dot`](Self::dot).
    pub fn undot(flat: &Map<String, Value>) -> Value {
        let mut value = Value::Object(Map::new());
        for (path, leaf) in flat {
            Self::set(&mut value, path, leaf.clone());
        }
        value
    }

    /// An object with only the given top-level keys.
    pub fn only(value: &Value, keys: &[&str]) -> Value {
        Value::Object(
            value
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(key, _)| keys.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )
    }

    /// An object without the given top-level keys.
    pub fn except(value: &Value, keys: &[&str]) -> Value {
        Value::Object(
            value
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(key, _)| !keys.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )
    }
}

/// Moves into `segment` of `current`, replacing anything that isn't the right kind of container
/// (a scalar, or an array reached by a non-numeric key) with an object.
fn step_into<'a>(current: &'a mut Value, segment: &str) -> &'a mut Value {
    let index = segment.parse::<usize>().ok();
    let indexes_array = current.is_array() && index.is_some();
    if !indexes_array && !current.is_object() {
        *current = Value::Object(Map::new());
    }
    match current {
        Value::Array(items) => {
            let index = index.unwrap_or_default();
            if items.len() <= index {
                items.resize(index + 1, Value::Null);
            }
            &mut items[index]
        }
        Value::Object(map) => map.entry(segment.to_owned()).or_insert(Value::Null),
        other => other,
    }
}

fn flatten(value: &Value, prefix: String, out: &mut Map<String, Value>) {
    let join = |key: &str| {
        if prefix.is_empty() {
            key.to_owned()
        } else {
            format!("{prefix}.{key}")
        }
    };
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                flatten(child, join(key), out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (index, child) in items.iter().enumerate() {
                flatten(child, join(&index.to_string()), out);
            }
        }
        leaf => {
            out.insert(prefix, leaf.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn get_and_has_walk_objects_and_arrays() {
        let data = json!({ "user": { "name": "Ann", "tags": ["a", "b"] }, "n": null });
        assert_eq!(Arr::get(&data, "user.name"), Some(&json!("Ann")));
        assert_eq!(Arr::get(&data, "user.tags.1"), Some(&json!("b")));
        assert_eq!(Arr::get(&data, "user.tags.9"), None);
        assert_eq!(Arr::get(&data, "user.name.x"), None);
        assert_eq!(Arr::get(&data, ""), Some(&data));
        assert!(Arr::has(&data, "n"));
        assert!(!Arr::has(&data, "missing.deep"));
    }

    #[test]
    fn set_creates_the_path() {
        let mut data = json!({});
        Arr::set(&mut data, "a.b.c", json!(1));
        Arr::set(&mut data, "a.d", json!("x"));
        assert_eq!(data, json!({ "a": { "b": { "c": 1 }, "d": "x" } }));

        // A scalar in the way is replaced; array indexes set and extend.
        Arr::set(&mut data, "a.d.e", json!(true));
        assert_eq!(Arr::get(&data, "a.d.e"), Some(&json!(true)));
        let mut list = json!({ "items": [1] });
        Arr::set(&mut list, "items.0", json!(9));
        Arr::set(&mut list, "items.2", json!(3));
        assert_eq!(list, json!({ "items": [9, null, 3] }));

        let mut whole = json!(1);
        Arr::set(&mut whole, "", json!(2));
        assert_eq!(whole, json!(2));
    }

    #[test]
    fn forget_removes_and_returns() {
        let mut data = json!({ "a": { "b": 1, "list": [10, 20] }, "c": 2 });
        assert_eq!(Arr::forget(&mut data, "a.b"), Some(json!(1)));
        assert_eq!(Arr::forget(&mut data, "a.list.0"), Some(json!(10)));
        assert_eq!(Arr::forget(&mut data, "c"), Some(json!(2)));
        assert_eq!(Arr::forget(&mut data, "a.nope.x"), None);
        assert_eq!(Arr::forget(&mut data, "a.list.5"), None);
        assert_eq!(data, json!({ "a": { "list": [20] } }));
    }

    #[test]
    fn pluck_dot_and_filters() {
        let rows = json!([{ "u": { "id": 1 } }, { "u": { "id": 2 } }, { "x": 1 }]);
        assert_eq!(Arr::pluck(&rows, "u.id"), [json!(1), json!(2)]);
        assert_eq!(Arr::pluck(&json!("not an array"), "id").len(), 0);

        let nested = json!({ "a": { "b": [1, { "c": 2 }] }, "empty": {}, "z": null });
        let flat = Arr::dot(&nested);
        assert_eq!(flat["a.b.0"], json!(1));
        assert_eq!(flat["a.b.1.c"], json!(2));
        assert_eq!(flat["empty"], json!({}));
        assert_eq!(flat["z"], json!(null));

        assert_eq!(Arr::only(&nested, &["z"]), json!({ "z": null }));
        assert_eq!(Arr::except(&nested, &["a", "empty"]), json!({ "z": null }));
        assert_eq!(Arr::only(&json!(1), &["a"]), json!({}));
    }

    #[test]
    fn undot_rebuilds_objects() {
        let mut flat = Map::new();
        flat.insert("a.b".into(), json!(1));
        flat.insert("a.c".into(), json!(2));
        assert_eq!(Arr::undot(&flat), json!({ "a": { "b": 1, "c": 2 } }));
    }
}
