//! Naming conventions shared by the generators.

use heck::{ToSnakeCase, ToUpperCamelCase};

use crate::error::CliError;

/// A Rust type name from user input: `comment`, `blog_post` and `BlogPost` all become
/// `Comment` / `BlogPost`.
pub fn type_name(input: &str) -> Result<String, CliError> {
    let valid = input
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && input.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err(CliError::invalid(format!(
            "`{input}` is not a valid name; use letters, digits and underscores, starting with a letter"
        )));
    }
    Ok(input.to_upper_camel_case())
}

/// `BlogPost` -> `blog_post`.
pub fn snake(name: &str) -> String {
    name.to_snake_case()
}

/// `post` -> `posts`, `category` -> `categories`, `box` -> `boxes`, `day` -> `days`.
pub fn pluralize(word: &str) -> String {
    if let Some(stem) = word
        .strip_suffix('y')
        .filter(|stem| !stem.ends_with(['a', 'e', 'i', 'o', 'u']))
    {
        format!("{stem}ies")
    } else if ["s", "x", "z", "ch", "sh"]
        .iter()
        .any(|suffix| word.ends_with(suffix))
    {
        format!("{word}es")
    } else {
        format!("{word}s")
    }
}

/// Names that can't be a new project: Rust keywords, standard crates and Oxide's own crates.
const RESERVED: &[&str] = &[
    "alloc",
    "core",
    "crate",
    "oxide",
    "proc-macro",
    "self",
    "std",
    "super",
    "test",
];

/// Checks a `oxide new` project name: kebab-case, usable as a Cargo package and a directory.
pub fn project_name(input: &str) -> Result<(), CliError> {
    let valid = input.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && input
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !input.ends_with('-')
        && !input.contains("--");
    if !valid {
        return Err(CliError::invalid(format!(
            "`{input}` is not a valid project name; use kebab-case like `my-oxide-app`"
        )));
    }
    if RESERVED.contains(&input) || input.starts_with("oxide-") {
        return Err(CliError::invalid(format!(
            "`{input}` is reserved; pick another project name"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names_are_pascal_case() {
        assert_eq!(type_name("comment").unwrap(), "Comment");
        assert_eq!(type_name("blog_post").unwrap(), "BlogPost");
        assert_eq!(type_name("BlogPost").unwrap(), "BlogPost");
        assert!(type_name("2fast").is_err());
        assert!(type_name("blog-post").is_err());
        assert!(type_name("").is_err());
    }

    #[test]
    fn plurals() {
        assert_eq!(pluralize("post"), "posts");
        assert_eq!(pluralize("category"), "categories");
        assert_eq!(pluralize("day"), "days");
        assert_eq!(pluralize("box"), "boxes");
        assert_eq!(pluralize("blog_post"), "blog_posts");
        assert_eq!(snake("BlogPost"), "blog_post");
    }

    #[test]
    fn project_names() {
        assert!(project_name("my-oxide-app").is_ok());
        assert!(project_name("blog2").is_ok());
        for bad in [
            "My-App",
            "my_app",
            "-app",
            "app-",
            "my--app",
            "2app",
            "",
            "oxide",
            "oxide-cli",
            "test",
        ] {
            assert!(project_name(bad).is_err(), "{bad} should be rejected");
        }
    }
}
