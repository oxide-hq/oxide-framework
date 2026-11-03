use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// The Oxide command line: scaffold applications and generate code.
#[derive(Debug, Parser)]
#[command(name = "oxide", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new Oxide application.
    New(NewArgs),
    /// Create a new Eloquent-style model (and optionally its migration).
    #[command(name = "make:model")]
    MakeModel(MakeModelArgs),
    /// Create a new controller.
    #[command(name = "make:controller")]
    MakeController(MakeControllerArgs),
    /// Create a new form request.
    #[command(name = "make:request")]
    MakeRequest(MakeRequestArgs),
    /// Create a new middleware
    #[command(name = "make:middleware")]
    MakeMiddleware(MakeMiddlewareArgs),
}

// Flags map one-to-one onto command-line switches.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Args)]
pub struct NewArgs {
    /// The project (and directory) name, in kebab-case.
    pub name: Option<String>,
    /// The application preset.
    #[arg(long, value_enum)]
    pub preset: Option<Preset>,
    /// The database driver (not for `--preset minimal`).
    #[arg(long, value_enum)]
    pub database: Option<Database>,
    /// The starter example (not for `--preset minimal`).
    #[arg(long, value_enum)]
    pub starter: Option<Starter>,
    /// Install the Oxide Eloquent Rust agent skill for Claude Code and Cursor.
    #[arg(long, overrides_with = "no_ai_skill")]
    pub ai_skill: bool,
    /// Don't install the agent skill.
    #[arg(long)]
    pub no_ai_skill: bool,
    /// Initialize a Git repository.
    #[arg(long)]
    pub git: bool,
    /// Never prompt; use flags and defaults.
    #[arg(short = 'n', long)]
    pub no_interaction: bool,
    /// Write into the directory even if it is not empty.
    #[arg(long)]
    pub force: bool,
    /// Depend on a local Oxide checkout (the `crates/oxide` directory) instead of GitHub.
    #[arg(long, value_name = "DIR")]
    pub oxide_path: Option<PathBuf>,
}

/// What kind of application to scaffold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Preset {
    /// JSON API: controllers, form requests, models and feature tests.
    Api,
    /// Routing and the service container only, without a database.
    Minimal,
}

/// The database driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Database {
    /// SQLite: zero-config, embedded.
    Sqlite,
    /// PostgreSQL.
    Postgres,
}

/// The example code to start from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Starter {
    /// A `Post` model, `PostController`, `StorePostRequest`, seeded migration and feature tests.
    Blog,
    /// A health-check endpoint only.
    Blank,
}

#[derive(Debug, Args)]
pub struct MakeModelArgs {
    /// The model name, e.g. `Comment`.
    pub name: String,
    /// Also create a migration for the model's table.
    #[arg(short, long)]
    pub migration: bool,
    /// Overwrite the file if it exists.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct MakeControllerArgs {
    /// The controller name, e.g. `Comment` or `CommentController`.
    pub name: String,
    /// Generate the five resource actions (`index`, `store`, `show`, `update`, `destroy`).
    #[arg(short, long)]
    pub resource: bool,
    /// Overwrite the file if it exists.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct MakeMiddlewareArgs {
    /// The middleware name, e.g. `EnsureRole`.
    pub name: String,
    /// Overwrite the file if it exists.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct MakeRequestArgs {
    /// The request name, e.g. `StoreCommentRequest`.
    pub name: String,
    /// Overwrite the file if it exists.
    #[arg(long)]
    pub force: bool,
}
