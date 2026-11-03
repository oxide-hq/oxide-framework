//! The console kernel: application commands that run inside the booted application, like
//! Artisan.
//!
//! ```ignore
//! struct SendMail;
//!
//! #[derive(clap::Parser)]
//! struct SendMailArgs { user: u64, #[arg(long)] dry_run: bool }
//!
//! impl Command for SendMail {
//!     fn name(&self) -> &'static str { "mail:send" }
//!     fn about(&self) -> &'static str { "Send the welcome mail" }
//!     fn handle<'a>(&'a self, ctx: &'a mut CommandContext<'_>) -> CommandFuture<'a> {
//!         Box::pin(async move {
//!             let args: SendMailArgs = ctx.parse()?;
//!             ctx.out.info(&format!("sending to {}", args.user));
//!             Ok(ExitCode::SUCCESS)
//!         })
//!     }
//! }
//!
//! let application = Application::builder().command(SendMail).build().await?;
//! std::process::exit(application.console().run(std::env::args().skip(1)).await);
//! ```

use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use clap::Parser;

use crate::provider::App;

/// What a command returns: its exit code (0 is success).
pub type ExitCode = i32;

pub type CommandFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ExitCode, CommandError>> + Send + 'a>>;

/// A command failed; the message is printed and the exit code is 1.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// `--help` or `--version`: printed to stdout, exit code 0.
    #[error("{0}")]
    Help(String),
    #[error("{0}")]
    Usage(String),
    #[error("{0}")]
    Failed(String),
}

impl From<clap::Error> for CommandError {
    fn from(error: clap::Error) -> Self {
        use clap::error::ErrorKind;
        match error.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => Self::Help(error.to_string()),
            _ => Self::Usage(error.to_string()),
        }
    }
}

/// A console command. `name` is what you type: `mail:send`.
pub trait Command: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn about(&self) -> &'static str {
        ""
    }

    fn handle<'a>(&'a self, ctx: &'a mut CommandContext<'_>) -> CommandFuture<'a>;
}

/// What a command receives: the application, its raw arguments and the output.
#[derive(Debug)]
pub struct CommandContext<'a> {
    pub app: &'a App,
    pub out: &'a mut Output,
    name: &'static str,
    args: Vec<String>,
}

impl CommandContext<'_> {
    /// Parses the arguments with a clap `Parser`: typed arguments and options, `--help` included.
    pub fn parse<P: Parser>(&self) -> Result<P, CommandError> {
        let argv = std::iter::once(self.name.to_owned()).chain(self.args.iter().cloned());
        Ok(P::try_parse_from(argv)?)
    }

    pub fn args(&self) -> &[String] {
        &self.args
    }
}

/// Text a command writes. Collected, so tests can read it; the console prints it at the end of
/// each command (and stderr lines to stderr).
#[derive(Debug, Default)]
pub struct Output {
    stdout: String,
    stderr: String,
}

impl Output {
    pub fn line(&mut self, text: &str) {
        let _ = writeln!(self.stdout, "{text}");
    }

    pub fn info(&mut self, text: &str) {
        self.line(text);
    }

    pub fn warn(&mut self, text: &str) {
        let _ = writeln!(self.stderr, "warning: {text}");
    }

    pub fn error(&mut self, text: &str) {
        let _ = writeln!(self.stderr, "error: {text}");
    }

    /// A left-aligned table.
    pub fn table(&mut self, headers: &[&str], rows: &[Vec<String>]) {
        let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
        for row in rows {
            for (width, cell) in widths.iter_mut().zip(row) {
                *width = (*width).max(cell.chars().count());
            }
        }
        let format_row = |cells: Vec<&str>| {
            cells
                .iter()
                .zip(&widths)
                .map(|(cell, width)| format!("{cell:<width$}"))
                .collect::<Vec<_>>()
                .join("  ")
                .trim_end()
                .to_owned()
        };
        self.line(&format_row(headers.to_vec()));
        self.line(
            &widths
                .iter()
                .map(|w| "-".repeat(*w))
                .collect::<Vec<_>>()
                .join("  "),
        );
        for row in rows {
            self.line(&format_row(row.iter().map(String::as_str).collect()));
        }
    }

    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    pub fn stderr(&self) -> &str {
        &self.stderr
    }
}

/// The result of [`Console::call`]: what a command printed and how it ended.
#[derive(Debug)]
pub struct CommandResult {
    pub exit_code: ExitCode,
    pub output: Output,
}

impl CommandResult {
    #[track_caller]
    pub fn assert_success(&self) -> &Self {
        assert_eq!(self.exit_code, 0, "stderr: {}", self.output.stderr());
        self
    }

    #[track_caller]
    pub fn assert_exit_code(&self, code: ExitCode) -> &Self {
        assert_eq!(self.exit_code, code, "stderr: {}", self.output.stderr());
        self
    }

    #[track_caller]
    pub fn assert_output_contains(&self, text: &str) -> &Self {
        assert!(
            self.output.stdout().contains(text),
            "stdout did not contain {text:?}:\n{}",
            self.output.stdout()
        );
        self
    }
}

/// Runs commands against a booted application. Built by `Application::console()`.
#[derive(Debug)]
pub struct Console {
    app: App,
    commands: Vec<Arc<dyn Command>>,
}

impl std::fmt::Debug for dyn Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Command({})", self.name())
    }
}

impl Console {
    pub fn new(app: App, commands: Vec<Arc<dyn Command>>) -> Self {
        Self { app, commands }
    }

    /// Runs the command named by the first argument and prints its output. Returns the exit
    /// code for `std::process::exit`. With no arguments, or `list`, prints the commands.
    pub async fn run<I, T>(&self, args: I) -> ExitCode
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        let result = self
            .execute(args.into_iter().map(Into::into).collect())
            .await;
        print!("{}", result.output.stdout());
        eprint!("{}", result.output.stderr());
        result.exit_code
    }

    /// Like [`run`](Self::run) but returns the output instead of printing it: for tests.
    /// `console.call("mail:send 42")` splits on whitespace.
    pub async fn call(&self, line: &str) -> CommandResult {
        self.execute(line.split_whitespace().map(str::to_owned).collect())
            .await
    }

    async fn execute(&self, mut args: Vec<String>) -> CommandResult {
        let mut output = Output::default();
        let name = if args.is_empty() {
            "list".to_owned()
        } else {
            args.remove(0)
        };
        let exit_code = if name == "list" {
            self.list(&mut output);
            0
        } else if let Some(command) = self.commands.iter().find(|c| c.name() == name) {
            let mut ctx = CommandContext {
                app: &self.app,
                out: &mut output,
                name: command.name(),
                args,
            };
            match command.handle(&mut ctx).await {
                Ok(code) => code,
                Err(CommandError::Help(text)) => {
                    output.line(text.trim_end());
                    0
                }
                Err(error) => {
                    output.error(&error.to_string());
                    1
                }
            }
        } else {
            output.error(&format!("Command \"{name}\" is not defined."));
            self.list(&mut output);
            1
        };
        CommandResult { exit_code, output }
    }

    fn list(&self, out: &mut Output) {
        let mut commands: Vec<&Arc<dyn Command>> = self.commands.iter().collect();
        commands.sort_by_key(|command| command.name());
        let rows: Vec<Vec<String>> = commands
            .iter()
            .map(|c| vec![c.name().to_owned(), c.about().to_owned()])
            .collect();
        out.line("Available commands:");
        out.table(&["command", "description"], &rows);
    }
}
