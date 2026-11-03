//! The application environment (`APP_ENV`).

use std::fmt;
use std::str::FromStr;

/// Where the application is running: `local`, `testing`, `staging`, `production`, or your own.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Environment {
    #[default]
    Local,
    Testing,
    Staging,
    Production,
    /// Any other name, stored lowercase.
    Custom(String),
}

impl Environment {
    /// The lowercase name, as written in `APP_ENV` and `.env.<name>`.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Local => "local",
            Self::Testing => "testing",
            Self::Staging => "staging",
            Self::Production => "production",
            Self::Custom(name) => name,
        }
    }

    pub fn is_local(&self) -> bool {
        *self == Self::Local
    }

    pub fn is_testing(&self) -> bool {
        *self == Self::Testing
    }

    pub fn is_production(&self) -> bool {
        *self == Self::Production
    }

    /// Whether this is any of `names` (case-insensitive), like Laravel's `environment('a', 'b')`.
    pub fn is_any(&self, names: &[&str]) -> bool {
        names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(self.as_str()))
    }
}

impl FromStr for Environment {
    type Err = std::convert::Infallible;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let name = raw.trim().to_ascii_lowercase();
        Ok(match name.as_str() {
            "" | "local" => Self::Local,
            "testing" | "test" => Self::Testing,
            "staging" => Self::Staging,
            "production" | "prod" => Self::Production,
            _ => Self::Custom(name),
        })
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse_case_insensitively_with_aliases() {
        assert_eq!("PRODUCTION".parse(), Ok(Environment::Production));
        assert_eq!("prod".parse(), Ok(Environment::Production));
        assert_eq!("test".parse(), Ok(Environment::Testing));
        assert_eq!("".parse(), Ok(Environment::Local));
        let qa: Environment = "QA".parse().unwrap();
        assert_eq!(qa, Environment::Custom("qa".into()));
        assert_eq!(qa.to_string(), "qa");
        assert!(qa.is_any(&["staging", "QA"]));
        assert!(!qa.is_production() && !qa.is_local() && !qa.is_testing());
    }
}
