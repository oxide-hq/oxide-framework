//! Boot-time configuration problems, reported all at once.

use std::fmt;

/// One problem found while loading configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigIssue {
    /// The section's file stem, e.g. `app`.
    pub section: String,
    /// The field, when the problem is about one.
    pub field: Option<String>,
    /// Where the offending value came from, e.g. `config/app.toml` or `$APP_PORT`.
    pub source: String,
    pub message: String,
}

impl fmt::Display for ConfigIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}", self.section)?;
        if let Some(field) = &self.field {
            write!(f, ".{field}")?;
        }
        write!(f, "] {} ({})", self.message, self.source)
    }
}

/// Every problem found while loading configuration.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub struct ConfigErrors {
    pub issues: Vec<ConfigIssue>,
}

/// `Debug` prints the readable list too, because `fn main() -> Result<..>` shows errors with it.
impl fmt::Debug for ConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid configuration ({} problem", self.issues.len())?;
        if self.issues.len() != 1 {
            f.write_str("s")?;
        }
        f.write_str("):")?;
        for issue in &self.issues {
            write!(f, "\n  - {issue}")?;
        }
        Ok(())
    }
}
