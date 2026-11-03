//! The `oxide` command.

use std::process::ExitCode;

use clap::Parser as _;
use oxide_cli::Cli;

fn main() -> ExitCode {
    match oxide_cli::run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{} {error}", console::style("error:").red().bold());
            ExitCode::FAILURE
        }
    }
}
