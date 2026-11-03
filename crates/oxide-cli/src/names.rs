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
    let name = input.to_upper_camel_case();
    if name == "Self" {
        return Err(CliError::invalid(
            "`Self` is a Rust keyword; pick another name",
        ));
    }
    Ok(name)
}

/// Rust 2024 strict and reserved keywords: none of them can be a module or crate name.
const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// The module (and file) name for a type: `BlogPost` -> `blog_post`. Rejects keywords such as
/// `type`, which can't follow `pub mod`, and `mod`, which would overwrite the parent `mod.rs`.
pub fn module_name(type_name: &str) -> Result<String, CliError> {
    let module = snake(type_name);
    if KEYWORDS.contains(&module.as_str()) {
        return Err(CliError::invalid(format!(
            "`{type_name}` would make a module named `{module}`, which is a Rust keyword; pick another name"
        )));
    }
    Ok(module)
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

/// Names that can't be a new project besides keywords: standard crates and Oxide's own crate.
const RESERVED: &[&str] = &["alloc", "core", "oxide", "proc-macro", "std", "test"];

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
    // The crate name (`my_app`) is what generated code writes, as in `use my_app::AppContext`.
    let crate_name = input.replace('-', "_");
    if RESERVED.contains(&input)
        || KEYWORDS.contains(&crate_name.as_str())
        || input.starts_with("oxide-")
    {
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
        assert!(type_name("self").is_err());
    }

    #[test]
    fn module_names_are_not_keywords() {
        assert_eq!(module_name("BlogPost").unwrap(), "blog_post");
        assert_eq!(module_name("TypeController").unwrap(), "type_controller");
        for keyword in ["Type", "Mod", "Async", "Fn", "Gen"] {
            assert!(
                module_name(keyword).is_err(),
                "{keyword} should be rejected"
            );
        }
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
            "async",
            "type",
            "self",
        ] {
            assert!(project_name(bad).is_err(), "{bad} should be rejected");
        }
    }
}
