use oxide::prelude::*;
use serde::Deserialize;

/// Extract it as `Validated<{{request}}>`: invalid input answers 422 before your handler runs.
#[derive(Debug, Deserialize, Validate, FormRequest)]
pub struct {{request}} {
    // #[validate(length(min = 1, message = "The name field is required."))]
    // pub name: String,
}
