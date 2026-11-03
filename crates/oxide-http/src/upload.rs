//! Multipart uploads: the [`Uploads`] extractor and [`UploadedFile`], like Laravel's
//! `$request->file('avatar')`.
//!
//! ```ignore
//! async fn store(uploads: Uploads) -> Result<StatusCode, OxideHttpError> {
//!     let avatar = uploads.file("avatar").ok_or_else(|| OxideHttpError::BadRequest("avatar required".into()))?;
//!     avatar.validate(&FileRules::new().max_kb(2048).mimes(&["png", "jpg"]))?;
//!     // storing to a disk arrives with the filesystem epic: for now use `avatar.bytes()`.
//!     Ok(StatusCode::CREATED)
//! }
//! ```

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{FromRequest, Multipart, Request};

use crate::error::OxideHttpError;

/// A file received in a multipart body, held in memory (bound the body with
/// `DefaultBodyLimit`).
#[derive(Debug, Clone)]
pub struct UploadedFile {
    name: String,
    mime: String,
    bytes: Bytes,
}

impl UploadedFile {
    pub fn new(name: impl Into<String>, mime: impl Into<String>, bytes: impl Into<Bytes>) -> Self {
        Self {
            name: name.into(),
            mime: mime.into(),
            bytes: bytes.into(),
        }
    }

    /// The client's file name, reduced to its last path component.
    pub fn name(&self) -> &str {
        self.name.rsplit(['/', '\\']).next().unwrap_or_default()
    }

    /// The lower-cased extension of the client's file name.
    pub fn extension(&self) -> Option<String> {
        let (stem, ext) = self.name().rsplit_once('.')?;
        (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_ascii_lowercase())
    }

    /// The content type the client declared.
    pub fn mime(&self) -> &str {
        &self.mime
    }

    pub fn size(&self) -> usize {
        self.bytes.len()
    }

    pub fn bytes(&self) -> &Bytes {
        &self.bytes
    }

    pub fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }

    /// Checks the file against `rules`; failures are a 422 on the field `field`.
    pub fn validate(&self, field: &'static str, rules: &FileRules) -> Result<(), OxideHttpError> {
        let fail = |message: String| {
            let mut errors = validator::ValidationErrors::new();
            let mut error = validator::ValidationError::new("file");
            error.message = Some(message.into());
            errors.add(field, error);
            Err(OxideHttpError::Validation(errors))
        };
        if let Some(max) = rules.max_kb
            && self.size() > max * 1024
        {
            return fail(format!(
                "The {field} must not be greater than {max} kilobytes."
            ));
        }
        if rules.image && !self.is_image() {
            return fail(format!("The {field} must be an image."));
        }
        if let Some(allowed) = &rules.mimes {
            let ext = self.extension().unwrap_or_default();
            if !allowed.contains(&ext) {
                return fail(format!(
                    "The {field} must be a file of type: {}.",
                    allowed.join(", ")
                ));
            }
        }
        Ok(())
    }
}

/// Constraints for [`UploadedFile::validate`]: Laravel's `file|image|mimes|max`.
#[derive(Debug, Clone, Default)]
pub struct FileRules {
    max_kb: Option<usize>,
    image: bool,
    mimes: Option<Vec<String>>,
}

impl FileRules {
    pub fn new() -> Self {
        Self::default()
    }

    /// Largest size in kilobytes.
    #[must_use]
    pub fn max_kb(mut self, kilobytes: usize) -> Self {
        self.max_kb = Some(kilobytes);
        self
    }

    #[must_use]
    pub fn image(mut self) -> Self {
        self.image = true;
        self
    }

    /// Allowed extensions, lower case, without the dot.
    #[must_use]
    pub fn mimes(mut self, extensions: &[&str]) -> Self {
        self.mimes = Some(extensions.iter().map(|e| (*e).to_owned()).collect());
        self
    }
}

/// The parsed parts of a `multipart/form-data` body.
#[derive(Debug, Default)]
pub struct Uploads {
    fields: HashMap<String, String>,
    files: HashMap<String, Vec<UploadedFile>>,
}

impl Uploads {
    /// A text field.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// The first file uploaded under `name`.
    pub fn file(&self, name: &str) -> Option<&UploadedFile> {
        self.files.get(name)?.first()
    }

    /// Every file uploaded under `name` (`photos[]` style repeats).
    pub fn files(&self, name: &str) -> &[UploadedFile] {
        self.files.get(name).map_or(&[], Vec::as_slice)
    }

    pub fn has_file(&self, name: &str) -> bool {
        self.file(name).is_some()
    }
}

impl<S: Send + Sync> FromRequest<S> for Uploads {
    type Rejection = OxideHttpError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let mut multipart = Multipart::from_request(request, state)
            .await
            .map_err(|error| OxideHttpError::BadRequest(error.body_text()))?;
        let mut uploads = Self::default();
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|error| OxideHttpError::BadRequest(error.body_text()))?
        {
            let name = field
                .name()
                .unwrap_or_default()
                .trim_end_matches("[]")
                .to_owned();
            let file_name = field.file_name().map(str::to_owned);
            let mime = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_owned();
            let bytes = field
                .bytes()
                .await
                .map_err(|error| OxideHttpError::with_status(error.status(), error.body_text()))?;
            match file_name {
                Some(file_name) if !file_name.is_empty() => uploads
                    .files
                    .entry(name)
                    .or_default()
                    .push(UploadedFile::new(file_name, mime, bytes)),
                _ => {
                    uploads
                        .fields
                        .insert(name, String::from_utf8_lossy(&bytes).into_owned());
                }
            }
        }
        Ok(uploads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_sanitised_and_rules_apply() {
        let file = UploadedFile::new("../../etc/Photo.PNG", "image/png", vec![0u8; 2048]);
        assert_eq!(file.name(), "Photo.PNG");
        assert_eq!(file.extension().as_deref(), Some("png"));
        assert!(file.is_image());
        assert!(
            file.validate(
                "avatar",
                &FileRules::new().image().mimes(&["png"]).max_kb(4)
            )
            .is_ok()
        );
        assert!(
            file.validate("avatar", &FileRules::new().max_kb(1))
                .is_err()
        );
        assert!(
            file.validate("avatar", &FileRules::new().mimes(&["pdf"]))
                .is_err()
        );
        let text = UploadedFile::new(".env", "text/plain", vec![1u8]);
        assert_eq!(text.extension(), None);
        assert!(text.validate("doc", &FileRules::new().image()).is_err());
    }
}
