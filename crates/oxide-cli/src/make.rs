//! `oxide make:*`: Artisan-style generators that create a file and register its module.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::{MakeControllerArgs, MakeMiddlewareArgs, MakeModelArgs, MakeRequestArgs};
use crate::error::CliError;
use crate::names;
use crate::templates::{self, render};

/// The database an application was scaffolded with, from `[package.metadata.oxide]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectDatabase {
    Sqlite,
    Postgres,
    None,
}

/// An Oxide application on disk.
#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub database: ProjectDatabase,
}

impl Project {
    /// Reads the application in `root`, which must have a `Cargo.toml` with
    /// `[package.metadata.oxide]` (as `oxide new` writes).
    pub fn discover(root: &Path) -> Result<Self, CliError> {
        let manifest_path = root.join("Cargo.toml");
        let not_an_app = || {
            CliError::invalid(format!(
                "{} is not an Oxide application (no [package.metadata.oxide] in Cargo.toml); \
                 run this inside a project created by `oxide new`",
                root.display()
            ))
        };
        let Ok(manifest) = fs::read_to_string(&manifest_path) else {
            return Err(not_an_app());
        };
        let manifest: toml::Table = manifest
            .parse()
            .map_err(|error| CliError::invalid(format!("{}: {error}", manifest_path.display())))?;
        let Some(metadata) = manifest
            .get("package")
            .and_then(|package| package.get("metadata"))
            .and_then(|metadata| metadata.get("oxide"))
        else {
            return Err(not_an_app());
        };
        let database = match metadata.get("database").and_then(toml::Value::as_str) {
            Some("sqlite") => ProjectDatabase::Sqlite,
            Some("postgres") => ProjectDatabase::Postgres,
            Some("none") | None => ProjectDatabase::None,
            Some(other) => {
                return Err(CliError::invalid(format!(
                    "unknown database `{other}` in [package.metadata.oxide]"
                )));
            }
        };
        Ok(Self {
            root: root.to_owned(),
            database,
        })
    }
}

/// `oxide make:model` in the current directory.
pub fn run_model(args: &MakeModelArgs) -> Result<(), CliError> {
    let project = Project::discover(&current_dir()?)?;
    let created = make_model(&project, &args.name, args.migration, args.force)?;
    report(&created);
    Ok(())
}

/// `oxide make:controller` in the current directory.
pub fn run_controller(args: &MakeControllerArgs) -> Result<(), CliError> {
    let project = Project::discover(&current_dir()?)?;
    let created = make_controller(&project, &args.name, args.resource, args.force)?;
    report(&created);
    Ok(())
}

/// `oxide make:middleware` in the current directory.
pub fn run_middleware(args: &MakeMiddlewareArgs) -> Result<(), CliError> {
    let project = Project::discover(&current_dir()?)?;
    let created = make_middleware(&project, &args.name, args.force)?;
    report(&created);
    Ok(())
}

/// `oxide make:request` in the current directory.
pub fn run_request(args: &MakeRequestArgs) -> Result<(), CliError> {
    let project = Project::discover(&current_dir()?)?;
    let created = make_request(&project, &args.name, args.force)?;
    report(&created);
    Ok(())
}

fn current_dir() -> Result<PathBuf, CliError> {
    std::env::current_dir().map_err(CliError::io("."))
}

/// What a generator created, for the `INFO` lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub kind: &'static str,
    /// Relative to the project root.
    pub path: PathBuf,
}

fn report(created: &[Created]) {
    for Created { kind, path } in created {
        println!(
            "  {} {kind} [{}] created successfully.",
            console::style(" INFO ").on_blue().white().bold(),
            path.display()
        );
    }
}

/// Creates `src/models/<name>.rs` (and with `migration`, a migration for its table) and
/// registers it in `src/models/mod.rs`.
pub fn make_model(
    project: &Project,
    name: &str,
    migration: bool,
    force: bool,
) -> Result<Vec<Created>, CliError> {
    if project.database == ProjectDatabase::None {
        return Err(CliError::invalid(
            "this application has no database (minimal preset), so it can't have models",
        ));
    }
    let model = names::type_name(name)?;
    let module = names::module_name(&model)?;
    let table = names::pluralize(&module);
    let vars = [("model", model.as_str()), ("table", table.as_str())];

    let mut planned = vec![(
        "Model",
        PathBuf::from(format!("src/models/{module}.rs")),
        render(templates::MODEL, &vars),
    )];
    if migration {
        let template = match project.database {
            ProjectDatabase::Postgres => templates::POSTGRES_TABLE,
            _ => templates::SQLITE_TABLE,
        };
        planned.push((
            "Migration",
            migration_path(&project.root, &table)?,
            render(template, &vars),
        ));
    }
    // Check every file before writing any, so a refusal leaves the project untouched.
    for (_, path, _) in &planned {
        refuse_existing(project, path, force)?;
    }

    let mut created = Vec::new();
    for (kind, path, contents) in planned {
        write_new(project, &path, &contents, true)?;
        created.push(Created { kind, path });
    }
    register_module(
        &project.root.join("src/models/mod.rs"),
        &format!("pub mod {module};"),
        Some(&format!("pub use {module}::{model};")),
    )?;
    Ok(created)
}

/// Where the `create_<table>_table` migration goes: the existing one if there is one (so
/// `--force` replaces it rather than adding a second `CREATE TABLE`), else the next number.
fn migration_path(root: &Path, table: &str) -> Result<PathBuf, CliError> {
    let dir = root.join("database/migrations");
    let suffix = format!("_create_{table}_table.sql");
    if dir.exists() {
        for entry in fs::read_dir(&dir).map_err(CliError::io(&dir))? {
            let name = entry.map_err(CliError::io(&dir))?.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(&suffix) {
                return Ok(PathBuf::from("database/migrations").join(name.as_ref()));
            }
        }
    }
    let number = next_migration_number(&dir)?;
    Ok(PathBuf::from(format!(
        "database/migrations/{number:04}{suffix}"
    )))
}

/// Creates `src/http/controllers/<name>_controller.rs` and registers it.
pub fn make_controller(
    project: &Project,
    name: &str,
    resource: bool,
    force: bool,
) -> Result<Vec<Created>, CliError> {
    let name = names::type_name(name)?;
    let base = name.strip_suffix("Controller").unwrap_or(&name);
    if base.is_empty() {
        return Err(CliError::invalid(
            "give the controller a name, e.g. `Comment`",
        ));
    }
    let controller = format!("{base}Controller");
    let module = names::module_name(&controller)?;
    let resource_path = names::pluralize(&names::snake(base));
    let vars = [
        ("controller", controller.as_str()),
        ("resource", resource_path.as_str()),
    ];
    let template = if resource {
        templates::RESOURCE_CONTROLLER
    } else {
        templates::CONTROLLER
    };

    let path = PathBuf::from(format!("src/http/controllers/{module}.rs"));
    write_new(project, &path, &render(template, &vars), force)?;
    register_module(
        &project.root.join("src/http/controllers/mod.rs"),
        &format!("pub mod {module};"),
        None,
    )?;
    Ok(vec![Created {
        kind: "Controller",
        path,
    }])
}

/// Creates `src/http/middleware/<name>.rs` and registers it, creating the `middleware` module
/// first in apps generated before the command existed.
pub fn make_middleware(
    project: &Project,
    name: &str,
    force: bool,
) -> Result<Vec<Created>, CliError> {
    let middleware = names::middleware_name(name)?;
    let module = names::module_name(&middleware)?;
    // Check the files that register the module exist before creating anything, so a failure
    // can't leave a middleware behind that nothing refers to.
    let http_mod = project.root.join("src/http/mod.rs");
    if !http_mod.is_file() {
        return Err(CliError::invalid(
            "src/http/mod.rs not found; run this inside an application created by `oxide new`",
        ));
    }
    let base = middleware
        .strip_suffix("Middleware")
        .filter(|base| !base.is_empty())
        .unwrap_or(&middleware);
    let alias = names::snake(base);
    let path = PathBuf::from(format!("src/http/middleware/{module}.rs"));
    write_new(
        project,
        &path,
        &render(
            templates::MIDDLEWARE,
            &[
                ("middleware", middleware.as_str()),
                ("alias", alias.as_str()),
            ],
        ),
        force,
    )?;
    let mod_file = project.root.join("src/http/middleware/mod.rs");
    if !mod_file.exists() {
        fs::write(&mod_file, templates::MIDDLEWARE_MOD).map_err(CliError::io(&mod_file))?;
    }
    register_module(&http_mod, "pub mod middleware;", None)?;
    register_module(&mod_file, &format!("pub mod {module};"), None)?;
    Ok(vec![Created {
        kind: "Middleware",
        path,
    }])
}

/// Creates `src/http/requests/<name>.rs` and registers it.
pub fn make_request(project: &Project, name: &str, force: bool) -> Result<Vec<Created>, CliError> {
    let request = names::type_name(name)?;
    let module = names::module_name(&request)?;
    let path = PathBuf::from(format!("src/http/requests/{module}.rs"));
    write_new(
        project,
        &path,
        &render(templates::REQUEST, &[("request", request.as_str())]),
        force,
    )?;
    register_module(
        &project.root.join("src/http/requests/mod.rs"),
        &format!("pub mod {module};"),
        None,
    )?;
    Ok(vec![Created {
        kind: "Request",
        path,
    }])
}

/// Fails if `relative` already exists under the project root, unless `force` is set.
fn refuse_existing(project: &Project, relative: &Path, force: bool) -> Result<(), CliError> {
    if !force && project.root.join(relative).exists() {
        return Err(CliError::invalid(format!(
            "{} already exists; use --force to overwrite it",
            relative.display()
        )));
    }
    Ok(())
}

/// Writes `relative` under the project root, refusing to overwrite unless `force` is set.
fn write_new(
    project: &Project,
    relative: &Path,
    contents: &str,
    force: bool,
) -> Result<(), CliError> {
    refuse_existing(project, relative, force)?;
    let path = project.root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(CliError::io(parent))?;
    }
    fs::write(&path, contents).map_err(CliError::io(&path))
}

/// The next `NNNN_` prefix in a migrations directory.
fn next_migration_number(dir: &Path) -> Result<u32, CliError> {
    if !dir.exists() {
        return Ok(1);
    }
    let mut highest = 0;
    for entry in fs::read_dir(dir).map_err(CliError::io(dir))? {
        let entry = entry.map_err(CliError::io(dir))?;
        let name = entry.file_name();
        let number = name
            .to_string_lossy()
            .split_once('_')
            .and_then(|(number, _)| number.parse::<u32>().ok());
        if let Some(number) = number {
            highest = highest.max(number);
        }
    }
    Ok(highest + 1)
}

/// Adds `mod_line` (and `use_line`) to the module file at `path` unless already there.
pub fn register_module(
    path: &Path,
    mod_line: &str,
    use_line: Option<&str>,
) -> Result<(), CliError> {
    let source = fs::read_to_string(path).map_err(CliError::io(path))?;
    let updated = with_module_lines(&source, mod_line, use_line);
    if updated != source {
        fs::write(path, updated).map_err(CliError::io(path))?;
    }
    Ok(())
}

/// `source` with `mod_line` after the last `pub mod` (or the module docs and inner attributes) and `use_line` after
/// the last `pub use` (or at the end). Lines already present are left alone.
pub fn with_module_lines(source: &str, mod_line: &str, use_line: Option<&str>) -> String {
    let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
    let has = |lines: &[String], wanted: &str| lines.iter().any(|line| line.trim() == wanted);

    if !has(&lines, mod_line) {
        if let Some(last) = lines.iter().rposition(|line| line.starts_with("pub mod ")) {
            lines.insert(item_end(&lines, last) + 1, mod_line.to_owned());
        } else {
            // Module docs and inner attributes (`#![allow(..)]`) must stay ahead of any item.
            let docs = lines
                .iter()
                .take_while(|line| line.starts_with("//!") || line.starts_with("#!["))
                .count();
            let mut block = vec![mod_line.to_owned()];
            if docs > 0 {
                block.insert(0, String::new());
            }
            if lines.len() > docs && !lines[docs].is_empty() {
                block.push(String::new());
            }
            lines.splice(docs..docs, block);
        }
    }
    if let Some(use_line) = use_line.filter(|line| !has(&lines, line)) {
        if let Some(last) = lines.iter().rposition(|line| line.starts_with("pub use ")) {
            lines.insert(item_end(&lines, last) + 1, use_line.to_owned());
        } else {
            if lines.last().is_some_and(|line| !line.is_empty()) {
                lines.push(String::new());
            }
            lines.push(use_line.to_owned());
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// The last line of the item starting at `start`, which may span lines like
/// `pub use post::{\n    NewPost,\n    Post,\n};` or an inline `pub mod x { .. }`.
fn item_end(lines: &[String], start: usize) -> usize {
    lines[start..]
        .iter()
        .position(|line| line.trim_end().ends_with(';') || line.starts_with('}'))
        .map_or(start, |offset| start + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_lines_go_next_to_their_kind() {
        let blog = "//! Models.\n\npub mod post;\n\npub use post::Post;\n";
        assert_eq!(
            with_module_lines(blog, "pub mod comment;", Some("pub use comment::Comment;")),
            "//! Models.\n\npub mod post;\npub mod comment;\n\npub use post::Post;\npub use comment::Comment;\n"
        );

        let empty = "//! Models.\n";
        assert_eq!(
            with_module_lines(empty, "pub mod comment;", Some("pub use comment::Comment;")),
            "//! Models.\n\npub mod comment;\n\npub use comment::Comment;\n"
        );
        assert_eq!(
            with_module_lines("", "pub mod comment;", None),
            "pub mod comment;\n"
        );

        let multiline = "pub mod post;\n\npub use post::{\n    NewPost,\n    Post,\n};\n";
        assert_eq!(
            with_module_lines(
                multiline,
                "pub mod comment;",
                Some("pub use comment::Comment;")
            ),
            "pub mod post;\npub mod comment;\n\npub use post::{\n    NewPost,\n    Post,\n};\npub use comment::Comment;\n"
        );

        let attributes = "//! Models.\n#![allow(dead_code)]\n";
        assert_eq!(
            with_module_lines(attributes, "pub mod comment;", None),
            "//! Models.\n#![allow(dead_code)]\n\npub mod comment;\n"
        );
    }

    #[test]
    fn registering_twice_changes_nothing() {
        let once = with_module_lines(
            "//! Models.\n",
            "pub mod comment;",
            Some("pub use comment::Comment;"),
        );
        assert_eq!(
            with_module_lines(&once, "pub mod comment;", Some("pub use comment::Comment;")),
            once
        );
    }
}
