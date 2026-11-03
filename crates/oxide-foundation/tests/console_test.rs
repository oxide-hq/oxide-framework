//! The console kernel through a booted application.

use clap::Parser;
use oxide_foundation::{
    Application, Command, CommandContext, CommandError, CommandFuture, ExitCode,
};

struct Greet;

#[derive(Parser)]
struct GreetArgs {
    name: String,
    #[arg(long, default_value_t = 1)]
    times: u8,
}

impl Command for Greet {
    fn name(&self) -> &'static str {
        "greet"
    }

    fn about(&self) -> &'static str {
        "Say hello"
    }

    fn handle<'a>(&'a self, ctx: &'a mut CommandContext<'_>) -> CommandFuture<'a> {
        Box::pin(async move {
            let args: GreetArgs = ctx.parse()?;
            for _ in 0..args.times {
                ctx.out.info(&format!("hello {}", args.name));
            }
            Ok::<ExitCode, CommandError>(0)
        })
    }
}

struct Fail;

impl Command for Fail {
    fn name(&self) -> &'static str {
        "fail"
    }

    fn handle<'a>(&'a self, _ctx: &'a mut CommandContext<'_>) -> CommandFuture<'a> {
        Box::pin(async { Err(CommandError::Failed("boom".into())) })
    }
}

async fn console() -> oxide_foundation::Console {
    Application::builder()
        .command(Greet)
        .command(Fail)
        .build()
        .await
        .unwrap()
        .console()
}

#[tokio::test]
async fn commands_parse_typed_arguments_and_print() {
    let console = console().await;
    console
        .call("greet Ada --times 2")
        .await
        .assert_success()
        .assert_output_contains("hello Ada\nhello Ada");
}

#[tokio::test]
async fn bad_arguments_failures_and_unknown_commands_exit_nonzero() {
    let console = console().await;
    console.call("greet").await.assert_exit_code(1);
    let failed = console.call("fail").await;
    failed.assert_exit_code(1);
    assert!(failed.output.stderr().contains("boom"));
    let unknown = console.call("nope").await;
    unknown.assert_exit_code(1);
    assert!(unknown.output.stderr().contains("not defined"));
    unknown.assert_output_contains("greet");
    console.call("greet --help").await.assert_exit_code(0);
}

#[tokio::test]
async fn list_shows_commands_with_descriptions() {
    let console = console().await;
    console
        .call("list")
        .await
        .assert_success()
        .assert_output_contains("greet")
        .assert_output_contains("Say hello");
    console.call("").await.assert_success();
}
