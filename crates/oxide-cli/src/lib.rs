//! # oxide-cli
//!
//! The `oxide` command, Oxide's answer to `laravel new` and `php artisan make:*`:
//!
//! - `oxide new <name>` scaffolds a ready-to-run application, through a guided wizard in a
//!   terminal or from flags (`--no-interaction`) in scripts and CI;
//! - `oxide make:model`, `make:controller` and `make:request` generate files inside an
//!   application and register them in the parent `mod.rs`.

mod cli;
mod error;
pub mod make;
pub mod names;
pub mod new;
mod templates;

pub use cli::{
    Cli, Command, Database, MakeControllerArgs, MakeModelArgs, MakeRequestArgs, NewArgs, Preset,
    Starter,
};
pub use error::CliError;

/// Runs a parsed command line.
pub fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::New(args) => new::run(args),
        Command::MakeModel(args) => make::run_model(&args),
        Command::MakeController(args) => make::run_controller(&args),
        Command::MakeRequest(args) => make::run_request(&args),
    }
}
