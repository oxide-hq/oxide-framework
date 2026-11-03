//! Turning environment variable strings into typed TOML values.

use std::path::PathBuf;

use toml::Value;

/// A type that can be read from an environment variable (`#[config(env = "...")]`).
///
/// Returns `Ok(None)` when the variable means "unset" (an empty `Option`). Implement it for your
/// own enums with [`impl_env_value_as_string!`](crate::impl_env_value_as_string).
pub trait EnvValue {
    /// The TOML value for `raw`, or a message saying what was wrong with it.
    fn from_env(raw: &str) -> Result<Option<Value>, String>;
}

impl EnvValue for bool {
    fn from_env(raw: &str) -> Result<Option<Value>, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "(true)" | "1" | "yes" | "on" => Ok(Some(Value::Boolean(true))),
            "false" | "(false)" | "0" | "no" | "off" => Ok(Some(Value::Boolean(false))),
            _ => Err("expected a boolean (true/false, 1/0, yes/no, on/off)".into()),
        }
    }
}

macro_rules! env_value_integer {
    ($($ty:ty),*) => {$(
        impl EnvValue for $ty {
            fn from_env(raw: &str) -> Result<Option<Value>, String> {
                let number: $ty = raw
                    .trim()
                    .parse()
                    .map_err(|_| concat!("expected an integer (", stringify!($ty), ")").to_owned())?;
                let number = i64::try_from(number)
                    .map_err(|_| "the number is too large".to_owned())?;
                Ok(Some(Value::Integer(number)))
            }
        }
    )*};
}
env_value_integer!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize);

macro_rules! env_value_float {
    ($($ty:ty),*) => {$(
        impl EnvValue for $ty {
            fn from_env(raw: &str) -> Result<Option<Value>, String> {
                let number: $ty = raw
                    .trim()
                    .parse()
                    .map_err(|_| concat!("expected a number (", stringify!($ty), ")").to_owned())?;
                Ok(Some(Value::Float(f64::from(number))))
            }
        }
    )*};
}
env_value_float!(f32, f64);

impl EnvValue for String {
    fn from_env(raw: &str) -> Result<Option<Value>, String> {
        Ok(Some(Value::String(raw.to_owned())))
    }
}

impl EnvValue for PathBuf {
    fn from_env(raw: &str) -> Result<Option<Value>, String> {
        Ok(Some(Value::String(raw.to_owned())))
    }
}

/// An empty variable, `null` or `(null)` is `None`.
impl<T: EnvValue> EnvValue for Option<T> {
    fn from_env(raw: &str) -> Result<Option<Value>, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "null" | "(null)" | "none" => Ok(None),
            _ => T::from_env(raw),
        }
    }
}

/// A comma-separated list: `a, b, c`.
impl<T: EnvValue> EnvValue for Vec<T> {
    fn from_env(raw: &str) -> Result<Option<Value>, String> {
        let mut items = Vec::new();
        for part in raw
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            if let Some(value) = T::from_env(part)? {
                items.push(value);
            }
        }
        Ok(Some(Value::Array(items)))
    }
}

/// Implements [`EnvValue`] for types that deserialize from a string, such as enums with
/// `#[serde(rename_all = "snake_case")]`.
#[macro_export]
macro_rules! impl_env_value_as_string {
    ($($ty:ty),* $(,)?) => {$(
        impl $crate::EnvValue for $ty {
            fn from_env(raw: &str) -> ::core::result::Result<::core::option::Option<$crate::__private::TomlValue>, ::std::string::String> {
                ::core::result::Result::Ok(::core::option::Option::Some(
                    $crate::__private::TomlValue::String(raw.to_owned()),
                ))
            }
        }
    )*};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalars_are_coerced() {
        assert_eq!(bool::from_env("Yes").unwrap(), Some(Value::Boolean(true)));
        assert_eq!(bool::from_env("0").unwrap(), Some(Value::Boolean(false)));
        assert!(bool::from_env("maybe").is_err());
        assert_eq!(u16::from_env(" 8080 ").unwrap(), Some(Value::Integer(8080)));
        assert!(u16::from_env("70000").is_err());
        assert!(u64::from_env("18446744073709551615").is_err());
        assert_eq!(f32::from_env("1.5").unwrap(), Some(Value::Float(1.5)));
        assert_eq!(
            String::from_env("a b").unwrap(),
            Some(Value::String("a b".into()))
        );
    }

    #[test]
    fn options_and_lists() {
        assert_eq!(<Option<u16>>::from_env("").unwrap(), None);
        assert_eq!(<Option<u16>>::from_env("(null)").unwrap(), None);
        assert_eq!(
            <Option<u16>>::from_env("5").unwrap(),
            Some(Value::Integer(5))
        );
        assert_eq!(
            <Vec<String>>::from_env("a, b,,c").unwrap(),
            Some(Value::Array(vec!["a".into(), "b".into(), "c".into()]))
        );
        assert!(<Vec<u8>>::from_env("1,x").is_err());
    }
}
