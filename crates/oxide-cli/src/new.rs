//! `oxide new`: the guided application installer.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, IsTerminal as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::cli::{Database, NewArgs, Preset, Starter};
use crate::error::CliError;
use crate::names;
use crate::templates::{self, render};

const OXIDE_GIT: &str = "https://github.com/oxide-hq/oxide-framework";

/// Where a new application gets the `oxide` crate from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OxideSource {
    /// The GitHub repository (the default: Oxide is not on crates.io yet).
    Git,
    /// A local checkout's `crates/oxide` directory.
    Path(PathBuf),
}

/// Everything `oxide new` needs to know, after flags and prompts are resolved.
#[derive(Debug, Clone)]
pub struct NewOptions {
    pub name: String,
    pub preset: Preset,
    /// `None` for the minimal preset.
    pub database: Option<Database>,
    pub starter: Starter,
    pub ai_skill: bool,
    pub git: bool,
    pub force: bool,
    pub oxide: OxideSource,
}

impl NewOptions {
    fn crate_name(&self) -> String {
        self.name.replace('-', "_")
    }
}

/// Runs `oxide new`: prompts in a terminal (unless `--no-interaction`), then scaffolds.
pub fn run(args: NewArgs) -> Result<(), CliError> {
    let interactive =
        !args.no_interaction && io::stdin().is_terminal() && io::stdout().is_terminal();
    let cwd = std::env::current_dir().map_err(CliError::io("."))?;
    let options = if interactive {
        cliclack::intro(
            console::style(format!(
                " Oxide Installer v{} — The Rust Framework for Web Artisans ",
                env!("CARGO_PKG_VERSION")
            ))
            .on_color256(166)
            .black()
            .bold(),
        )
        .map_err(CliError::prompt)?;
        prompt_options(args, &cwd)?
    } else {
        flag_options(args)?
    };
    let dest = cwd.join(&options.name);
    check_destination(&dest, options.force)?;

    let started = Instant::now();
    let spinner = interactive.then(|| {
        let spinner = cliclack::spinner();
        spinner.start("Scaffolding your application...");
        spinner
    });
    let written = match scaffold(&options, &dest) {
        Ok(written) => written,
        Err(error) => {
            if let Some(spinner) = spinner {
                spinner.error("Scaffolding failed");
            }
            return Err(error);
        }
    };
    let summary = format!(
        "Scaffolded {} files in ./{} ({:.1?})",
        written.len(),
        options.name,
        started.elapsed()
    );
    let git = options.git.then(|| git_init(&dest));

    let mut next = format!("$ cd {}\n", options.name);
    if options.database == Some(Database::Postgres) {
        next.push_str("$ cp .env.example .env   # then point DATABASE_URL at your server\n");
    }
    next.push_str("$ cargo test\n$ cargo run");

    let Some(spinner) = spinner else {
        println!("{summary}");
        if let Some(Err(warning)) = git {
            eprintln!("warning: {warning}");
        }
        println!("\nApplication ready! Build something amazing:\n");
        for line in next.lines() {
            println!("   {line}");
        }
        return Ok(());
    };
    spinner.stop(summary);
    if let Some(Err(warning)) = git {
        cliclack::log::warning(warning).map_err(CliError::prompt)?;
    }
    let next = next.replace('\n', "\n   ");
    cliclack::outro(format!(
        "Application ready! Build something amazing:\n   {next}"
    ))
    .map_err(CliError::prompt)
}

/// Resolves options from flags and defaults, without prompting.
pub fn flag_options(args: NewArgs) -> Result<NewOptions, CliError> {
    let Some(name) = args.name.clone() else {
        return Err(CliError::invalid(
            "a project name is required with --no-interaction (or outside a terminal)",
        ));
    };
    names::project_name(&name)?;
    let preset = args.preset.unwrap_or(Preset::Api);
    let (database, starter) = match preset {
        Preset::Minimal => minimal_choices(&args)?,
        Preset::Api => (
            Some(args.database.unwrap_or(Database::Sqlite)),
            args.starter.unwrap_or(Starter::Blog),
        ),
    };
    Ok(NewOptions {
        name,
        preset,
        database,
        starter,
        ai_skill: !args.no_ai_skill,
        git: args.git,
        force: args.force,
        oxide: oxide_source(args.oxide_path)?,
    })
}

/// The minimal preset has no database, so it rejects database-related flags.
fn minimal_choices(args: &NewArgs) -> Result<(Option<Database>, Starter), CliError> {
    if args.database.is_some() || args.starter == Some(Starter::Blog) {
        return Err(CliError::invalid(
            "the minimal preset has no database; drop --database and --starter blog, or use --preset api",
        ));
    }
    Ok((None, Starter::Blank))
}

/// Resolves options by prompting for everything the flags left open.
fn prompt_options(args: NewArgs, cwd: &Path) -> Result<NewOptions, CliError> {
    let name = if let Some(name) = args.name.clone() {
        names::project_name(&name)?;
        name
    } else {
        let cwd = cwd.to_owned();
        let force = args.force;
        cliclack::input("What is the name of your project?")
            .placeholder("my-oxide-app")
            .validate(move |input: &String| {
                names::project_name(input)?;
                check_destination(&cwd.join(input), force)
            })
            .interact::<String>()
            .map_err(CliError::prompt)?
    };

    let preset = match args.preset {
        Some(preset) => preset,
        None => cliclack::select("Which application preset would you like to install?")
            .item(
                Preset::Api,
                "RESTful JSON API",
                "Controllers, FormRequests, Eloquent ORM, Feature Tests",
            )
            .item(
                Preset::Minimal,
                "Minimal Skeleton",
                "Routing & Service Container only",
            )
            .interact()
            .map_err(CliError::prompt)?,
    };

    let (database, starter) = if preset == Preset::Minimal {
        minimal_choices(&args)?
    } else {
        let database = match args.database {
            Some(database) => database,
            None => cliclack::select("Which database driver will your application use?")
                .item(
                    Database::Sqlite,
                    "SQLite",
                    "Zero-config embedded database — instant local dev",
                )
                .item(
                    Database::Postgres,
                    "PostgreSQL",
                    "Production-ready relational database",
                )
                .interact()
                .map_err(CliError::prompt)?,
        };
        let starter = if let Some(starter) = args.starter {
            starter
        } else {
            let blog = cliclack::confirm(
                    "Would you like to include the Blog Starter example (Post Model, CRUD Controller & Feature Tests)?",
                )
                .initial_value(true)
                .interact()
                .map_err(CliError::prompt)?;
            if blog { Starter::Blog } else { Starter::Blank }
        };
        (Some(database), starter)
    };

    let ai_skill = if args.ai_skill || args.no_ai_skill {
        args.ai_skill
    } else {
        cliclack::confirm(
            "Install the Oxide Eloquent Rust AI Skill (.claude/skills & .cursor/rules)?",
        )
        .initial_value(true)
        .interact()
        .map_err(CliError::prompt)?
    };

    Ok(NewOptions {
        name,
        preset,
        database,
        starter,
        ai_skill,
        git: args.git,
        force: args.force,
        oxide: oxide_source(args.oxide_path)?,
    })
}

fn oxide_source(path: Option<PathBuf>) -> Result<OxideSource, CliError> {
    let Some(path) = path else {
        return Ok(OxideSource::Git);
    };
    let path = fs::canonicalize(&path).map_err(CliError::io(&path))?;
    if !path.join("Cargo.toml").is_file() {
        return Err(CliError::invalid(format!(
            "{} has no Cargo.toml; --oxide-path takes the `crates/oxide` directory of a checkout",
            path.display()
        )));
    }
    Ok(OxideSource::Path(path))
}

/// Fails if `dest` exists and is not an empty directory, unless `force` is set.
fn check_destination(dest: &Path, force: bool) -> Result<(), CliError> {
    let display = dest.file_name().map_or_else(
        || dest.display().to_string(),
        |name| format!("./{}", name.to_string_lossy()),
    );
    if dest.is_file() {
        return Err(CliError::invalid(format!(
            "{display} already exists and is a file"
        )));
    }
    if force || !dest.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(dest).map_err(CliError::io(dest))?;
    if entries.next().is_some() {
        return Err(CliError::invalid(format!(
            "{display} already exists and is not empty; use --force to write into it"
        )));
    }
    Ok(())
}

fn git_init(dest: &Path) -> Result<(), String> {
    match Command::new("git")
        .args(["init", "-q"])
        .current_dir(dest)
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("`git init` failed ({status})")),
        Err(error) => Err(format!("could not run git: {error}")),
    }
}

/// Writes the application into `dest` and returns the files written, relative to `dest`.
pub fn scaffold(options: &NewOptions, dest: &Path) -> Result<Vec<PathBuf>, CliError> {
    let files = files(options);
    for (path, contents) in &files {
        let path = dest.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(CliError::io(parent))?;
        }
        fs::write(&path, contents).map_err(CliError::io(&path))?;
    }
    Ok(files
        .into_iter()
        .map(|(path, _)| PathBuf::from(path))
        .collect())
}

/// The pieces of the generated app that depend on whether it has a database: the health test's
/// context (which also applies migrations), and the `DatabaseConfig` section with its loader line.
fn config_parts(
    database: Option<Database>,
    crate_name: &str,
) -> (String, &'static str, &'static str) {
    // With a database, apply the migrations too, so a broken one fails the test suite.
    let health_context = if database.is_some() {
        format!(
            "{{\n        let db = DatabasePool::connect_fresh(&database_url()).await.unwrap();\n        \
             db.migrate(&{crate_name}::MIGRATOR).await.unwrap();\n        \
             AppContext::new(db, {crate_name}::config::load().unwrap())\n    }}"
        )
    } else {
        format!("AppContext::new({crate_name}::config::load().unwrap())")
    };
    let (database_config, database_section) = if database.is_some() {
        (
            concat!(
                "\n/// `config/database.toml`. The URL may hold a password, so `Debug` hides it.\n",
                "#[derive(Clone, Deserialize, Config)]\n",
                "#[config(file = \"database\")]\n",
                "pub struct DatabaseConfig {\n",
                "    #[config(env = \"DATABASE_URL\", secret)]\n",
                "    pub url: String,\n",
                "}\n",
            ),
            "\n        .section::<DatabaseConfig>()",
        )
    } else {
        ("", "")
    };
    (health_context, database_config, database_section)
}

/// Every file of the new application, as `(relative path, contents)`.
fn files(options: &NewOptions) -> Vec<(String, String)> {
    let crate_name = options.crate_name();
    let database = options.database;
    let default_database_url = match database {
        Some(Database::Sqlite) => "sqlite://database/database.sqlite?mode=rwc".to_owned(),
        Some(Database::Postgres) => {
            format!("postgres://postgres:postgres@localhost:5432/{crate_name}")
        }
        None => String::new(),
    };
    let database_url_fn = match database {
        Some(Database::Sqlite) => concat!(
            "fn database_url() -> String {\n",
            "    // Every test gets its own in-memory database.\n",
            "    \"sqlite::memory:\".to_owned()\n",
            "}\n",
        ),
        Some(Database::Postgres) => concat!(
            "fn database_url() -> String {\n",
            "    // Every test gets its own schema on this server (see `DatabasePool::connect_fresh`).\n",
            "    dotenvy::dotenv().ok();\n",
            "    std::env::var(\"DATABASE_URL\")\n",
            "        .expect(\"set DATABASE_URL (or copy .env.example to .env) to run the tests\")\n",
            "}\n",
        ),
        None => "",
    };
    let (database_notes, data_rows) = match database {
        Some(Database::Sqlite) => (
            "\nSQLite needs no setup: the database file is created in `database/` on first run.\n",
            "| `src/models/` | Models (`#[derive(Model)]`) |\n| `database/migrations/` | Numbered SQL migrations, applied at startup |\n",
        ),
        Some(Database::Postgres) => (
            "\nCreate the database named in `DATABASE_URL` first. The tests use the same server, with a\nthrowaway schema for each test.\n",
            "| `src/models/` | Models (`#[derive(Model)]`) |\n| `database/migrations/` | Numbered SQL migrations, applied at startup |\n",
        ),
        None => ("", ""),
    };
    let (health_context, database_config, database_section) = config_parts(database, &crate_name);
    let vars = [
        ("name", options.name.as_str()),
        ("crate_name", crate_name.as_str()),
        ("default_database_url", default_database_url.as_str()),
        ("mailer", templates::MAILER.trim_end()),
        ("database_url_fn", database_url_fn),
        ("health_context", health_context.as_str()),
        ("database_config", database_config),
        ("database_section", database_section),
        ("database_notes", database_notes),
        ("data_rows", data_rows),
    ];
    let r = |template: &str| render(template, &vars);

    let mut files = vec![
        ("Cargo.toml".to_owned(), cargo_toml(options)),
        (
            ".env.example".to_owned(),
            env_example(options, &default_database_url),
        ),
        (".gitignore".to_owned(), r(templates::GITIGNORE)),
        ("README.md".to_owned(), r(templates::README)),
        ("src/http/mod.rs".to_owned(), r(templates::HTTP_MOD)),
        (
            "src/http/middleware/mod.rs".to_owned(),
            templates::MIDDLEWARE_MOD.to_owned(),
        ),
        ("src/routes/mod.rs".to_owned(), r(templates::ROUTES_MOD)),
    ];
    let mut add = |path: &str, contents: String| files.push((path.to_owned(), contents));

    add("src/config.rs", r(templates::CONFIG_RS));
    add("config/app.toml", r(templates::CONFIG_APP));
    add("config/logging.toml", r(templates::CONFIG_LOGGING));
    if database.is_some() {
        add("config/database.toml", r(templates::CONFIG_DATABASE));
        add("build.rs", r(templates::BUILD_RS));
        add("src/main.rs", r(templates::MAIN));
        add("src/lib.rs", r(templates::LIB));
        add("src/context.rs", r(templates::CONTEXT));
    } else {
        add("src/main.rs", r(templates::MAIN_MINIMAL));
        add("src/lib.rs", r(templates::LIB_MINIMAL));
        add("src/context.rs", r(templates::CONTEXT_MINIMAL));
    }

    starter_files(options, &r, &mut add);

    if options.ai_skill {
        let skill = templates::app_skill();
        add(
            ".cursor/rules/oxide-eloquent-rust.mdc",
            templates::cursor_rule(&skill),
        );
        add(".claude/skills/oxide-eloquent-rust/SKILL.md", skill);
    }
    files
}

/// The files that differ between the blog and blank starters.
fn starter_files(
    options: &NewOptions,
    r: &dyn Fn(&str) -> String,
    add: &mut dyn FnMut(&str, String),
) {
    let database = options.database;
    let controllers = "//! Controllers. Generate one with `oxide make:controller`.\n";
    let requests = "//! Form requests. Generate one with `oxide make:request`.\n";
    let models = "//! Models. Generate one with `oxide make:model`.\n";
    if options.starter == Starter::Blog {
        add("src/routes/api.rs", r(templates::API_BLOG));
        add(
            "src/models/mod.rs",
            format!("{models}\npub mod post;\n\npub use post::Post;\n"),
        );
        add("src/models/post.rs", r(templates::POST_MODEL));
        add(
            "src/http/controllers/mod.rs",
            format!("{controllers}\npub mod post_controller;\n"),
        );
        add(
            "src/http/controllers/post_controller.rs",
            r(templates::POST_CONTROLLER),
        );
        add(
            "src/http/requests/mod.rs",
            format!("{requests}\npub mod store_post_request;\n"),
        );
        add(
            "src/http/requests/store_post_request.rs",
            r(templates::STORE_POST_REQUEST),
        );
        let migration = match database {
            Some(Database::Postgres) => templates::POSTGRES_POSTS,
            _ => templates::SQLITE_POSTS,
        };
        add(
            "database/migrations/0001_create_posts_table.sql",
            r(migration),
        );
        add(
            "tests/post_feature_test.rs",
            r(templates::POST_FEATURE_TEST),
        );
    } else {
        add("src/routes/api.rs", r(templates::API_BLANK));
        add("src/http/controllers/mod.rs", controllers.to_owned());
        add("src/http/requests/mod.rs", requests.to_owned());
        if database.is_some() {
            add("src/models/mod.rs", models.to_owned());
            add("database/migrations/.gitkeep", String::new());
        }
        add("tests/health_test.rs", r(templates::HEALTH_TEST));
    }
}

fn cargo_toml(options: &NewOptions) -> String {
    let source = match &options.oxide {
        OxideSource::Git => format!("git = \"{OXIDE_GIT}\""),
        OxideSource::Path(path) => {
            format!("path = {}", toml::Value::String(path.display().to_string()))
        }
    };
    let features = if options.database == Some(Database::Postgres) {
        ", features = [\"postgres\"]"
    } else {
        ""
    };
    let sqlx = if options.database.is_some() {
        "sqlx = { version = \"0.8\", default-features = false, features = [\"runtime-tokio\", \"macros\", \"migrate\"] }\n"
    } else {
        ""
    };
    let (preset, database) = match (options.preset, options.database) {
        (Preset::Minimal, _) | (Preset::Api, None) => ("minimal", "none"),
        (Preset::Api, Some(Database::Sqlite)) => ("api", "sqlite"),
        (Preset::Api, Some(Database::Postgres)) => ("api", "postgres"),
    };
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
rust-version = "1.88"
publish = false

[dependencies]
oxide = {{ {source}{features} }}
tokio = {{ version = "1", features = ["full"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
validator = {{ version = "0.21", features = ["derive"] }}
{sqlx}dotenvy = "0.15"

# Read by `oxide make:*`.
[package.metadata.oxide]
preset = "{preset}"
database = "{database}"
"#,
        name = options.name,
    )
}

fn env_example(options: &NewOptions, database_url: &str) -> String {
    let mut env = format!(
        "APP_NAME={}\nAPP_ENV=local\nAPP_DEBUG=false\nAPP_PORT=3000\nLOG_LEVEL=info\n",
        options.name
    );
    if options.database.is_some() {
        let _ = writeln!(env, "DATABASE_URL={database_url}");
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(preset: Preset, database: Option<Database>, starter: Starter) -> NewOptions {
        NewOptions {
            name: "sample-app".into(),
            preset,
            database,
            starter,
            ai_skill: true,
            git: false,
            force: false,
            oxide: OxideSource::Git,
        }
    }

    fn paths(options: &NewOptions) -> Vec<String> {
        files(options).into_iter().map(|(path, _)| path).collect()
    }

    #[test]
    fn no_placeholder_survives_rendering() {
        for options in [
            options(Preset::Api, Some(Database::Sqlite), Starter::Blog),
            options(Preset::Api, Some(Database::Postgres), Starter::Blog),
            options(Preset::Api, Some(Database::Sqlite), Starter::Blank),
            options(Preset::Minimal, None, Starter::Blank),
        ] {
            for (path, contents) in files(&options) {
                assert!(
                    !contents.contains("{{"),
                    "{path} has an unrendered placeholder"
                );
            }
        }
    }

    #[test]
    fn presets_choose_their_files() {
        let blog = paths(&options(Preset::Api, Some(Database::Sqlite), Starter::Blog));
        assert!(blog.contains(&"tests/post_feature_test.rs".to_owned()));
        assert!(blog.contains(&"database/migrations/0001_create_posts_table.sql".to_owned()));

        let minimal = paths(&options(Preset::Minimal, None, Starter::Blank));
        assert!(minimal.contains(&"tests/health_test.rs".to_owned()));
        assert!(
            !minimal
                .iter()
                .any(|path| path.starts_with("database/") || path == "build.rs")
        );
    }

    #[test]
    fn cargo_toml_reflects_the_database() {
        let mut postgres = options(Preset::Api, Some(Database::Postgres), Starter::Blog);
        postgres.oxide = OxideSource::Path(PathBuf::from("/src/oxide/crates/oxide"));
        let manifest = cargo_toml(&postgres);
        assert!(
            manifest.contains(
                r#"oxide = { path = "/src/oxide/crates/oxide", features = ["postgres"] }"#
            )
        );
        assert!(manifest.contains("database = \"postgres\""));
        let parsed: toml::Table = manifest.parse().unwrap();
        assert_eq!(parsed["package"]["name"].as_str(), Some("sample-app"));

        let minimal = cargo_toml(&options(Preset::Minimal, None, Starter::Blank));
        assert!(minimal.contains(&format!("oxide = {{ git = \"{OXIDE_GIT}\" }}")));
        assert!(!minimal.contains("sqlx"));
        assert!(minimal.contains("database = \"none\""));
    }
}
