use oxide::prelude::*;
use serde::Serialize;

/// Add a field for each column of the `{{table}}` table.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, Model)]
#[model(table = "{{table}}")]
pub struct {{model}} {
    pub id: i64,
}
