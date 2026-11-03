//! [`Context`]: key/values that follow a request through every log line.
//!
//! A context lives for one task (a request, a job): [`Context::scope`] starts one, and anything
//! logged inside it carries the visible values. Hidden values travel with the context but never
//! appear in logs, for data such as a tenant id that a queued job needs to restore.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;

#[derive(Debug, Clone, Default)]
struct Data {
    visible: BTreeMap<String, Value>,
    hidden: BTreeMap<String, Value>,
}

tokio::task_local! {
    static CURRENT: Arc<Mutex<Data>>;
}

/// Reads and writes the current task's context. All methods are no-ops (or `None`) outside a
/// [`Context::scope`].
#[derive(Debug, Clone, Copy)]
pub struct Context;

impl Context {
    /// Runs `future` with its own context. A scope started inside another one begins with a
    /// copy of the outer values, and changes inside it don't leak back out.
    pub async fn scope<F: Future>(future: F) -> F::Output {
        let inherited = CURRENT
            .try_with(|data| lock(data).clone())
            .unwrap_or_default();
        CURRENT.scope(Arc::new(Mutex::new(inherited)), future).await
    }

    /// Like [`scope`](Self::scope), but starting from `dehydrated` (see
    /// [`dehydrate`](Self::dehydrate)), to restore a context in another task or process.
    pub async fn scope_from<F: Future>(dehydrated: &Value, future: F) -> F::Output {
        CURRENT
            .scope(Arc::new(Mutex::new(rehydrate(dehydrated))), future)
            .await
    }

    /// Adds a value to every log line from now on. Returns `false` outside a scope.
    pub fn add(key: impl Into<String>, value: impl Into<Value>) -> bool {
        Self::with_data(|data| {
            data.visible.insert(key.into(), value.into());
        })
    }

    /// Adds a value that is carried but never logged. Returns `false` outside a scope.
    pub fn add_hidden(key: impl Into<String>, value: impl Into<Value>) -> bool {
        Self::with_data(|data| {
            data.hidden.insert(key.into(), value.into());
        })
    }

    /// A visible value.
    pub fn get(key: &str) -> Option<Value> {
        Self::read(|data| data.visible.get(key).cloned()).flatten()
    }

    /// A hidden value.
    pub fn get_hidden(key: &str) -> Option<Value> {
        Self::read(|data| data.hidden.get(key).cloned()).flatten()
    }

    /// Removes a visible or hidden value.
    pub fn forget(key: &str) -> bool {
        Self::with_data(|data| {
            data.visible.remove(key);
            data.hidden.remove(key);
        })
    }

    /// Every visible value: what log lines carry.
    pub fn all() -> BTreeMap<String, Value> {
        Self::read(|data| data.visible.clone()).unwrap_or_default()
    }

    /// The whole context, hidden values included, as JSON that [`scope_from`](Self::scope_from)
    /// restores: `{"visible": {..}, "hidden": {..}}`.
    pub fn dehydrate() -> Value {
        Self::read(|data| serde_json::json!({ "visible": data.visible, "hidden": data.hidden }))
            .unwrap_or_else(|| serde_json::json!({ "visible": {}, "hidden": {} }))
    }

    /// Runs `read` on the visible values without copying them (used for every log line).
    pub(crate) fn with_visible<R>(read: impl FnOnce(&BTreeMap<String, Value>) -> R) -> Option<R> {
        Self::read(|data| read(&data.visible))
    }

    fn with_data(change: impl FnOnce(&mut Data)) -> bool {
        CURRENT.try_with(|data| change(&mut lock(data))).is_ok()
    }

    fn read<T>(read: impl FnOnce(&Data) -> T) -> Option<T> {
        CURRENT.try_with(|data| read(&lock(data))).ok()
    }
}

fn lock(data: &Mutex<Data>) -> std::sync::MutexGuard<'_, Data> {
    data.lock().unwrap_or_else(PoisonError::into_inner)
}

fn rehydrate(value: &Value) -> Data {
    let map = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_object)
            .map(|object| object.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    };
    Data {
        visible: map("visible"),
        hidden: map("hidden"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn values_live_inside_a_scope_only() {
        assert!(!Context::add("k", 1));
        assert_eq!(Context::get("k"), None);
        assert!(Context::all().is_empty());

        Context::scope(async {
            assert!(Context::add("request_id", "abc"));
            assert!(Context::add_hidden("tenant", 7));
            assert_eq!(Context::get("request_id"), Some(json!("abc")));
            assert_eq!(Context::get("tenant"), None, "hidden values aren't visible");
            assert_eq!(Context::get_hidden("tenant"), Some(json!(7)));
            assert_eq!(Context::all().len(), 1);
            assert!(Context::forget("tenant"));
            assert_eq!(Context::get_hidden("tenant"), None);
        })
        .await;
        assert_eq!(Context::get("request_id"), None);
    }

    #[tokio::test]
    async fn nested_scopes_inherit_a_copy() {
        Context::scope(async {
            Context::add("outer", 1);
            Context::scope(async {
                assert_eq!(Context::get("outer"), Some(json!(1)));
                Context::add("inner", 2);
                Context::add("outer", 99);
            })
            .await;
            assert_eq!(Context::get("outer"), Some(json!(1)));
            assert_eq!(Context::get("inner"), None);
        })
        .await;
    }

    #[tokio::test]
    async fn dehydrate_and_restore_keep_hidden_values() {
        let snapshot = Context::scope(async {
            Context::add("request_id", "r1");
            Context::add_hidden("tenant", 7);
            Context::dehydrate()
        })
        .await;
        assert_eq!(
            snapshot,
            json!({ "visible": { "request_id": "r1" }, "hidden": { "tenant": 7 } })
        );

        Context::scope_from(&snapshot, async {
            assert_eq!(Context::get("request_id"), Some(json!("r1")));
            assert_eq!(Context::get_hidden("tenant"), Some(json!(7)));
        })
        .await;
        assert_eq!(Context::dehydrate(), json!({ "visible": {}, "hidden": {} }));
    }

    #[tokio::test]
    async fn a_context_is_per_task() {
        Context::scope(async {
            Context::add("mine", 1);
            let other = tokio::spawn(async { Context::get("mine") }).await.unwrap();
            assert_eq!(other, None, "spawned tasks don't inherit a context");
        })
        .await;
    }
}
