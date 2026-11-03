use oxide::prelude::*;
use serde::Deserialize;

/// The body of `POST /posts` and `PUT /posts/{post}`. Invalid input never reaches the
/// controller: `Validated<StorePostRequest>` answers 422 with Laravel-shaped errors.
#[derive(Debug, Deserialize, Validate, FormRequest)]
pub struct StorePostRequest {
    #[validate(length(min = 3, max = 255, message = "The title must be between 3 and 255 characters."))]
    pub title: String,
    #[validate(length(min = 1, message = "The body field is required."))]
    pub body: String,
    #[serde(default)]
    pub published: bool,
}
