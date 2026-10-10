//! `brolga-cli`: the client's commands from a shell, and the MCP server.
//!
//! When the app is running, commands are passed to it over the control
//! channel, because only one process may own an account's data at a time.
//! When it is not, they run here.

use std::process::ExitCode;
use std::sync::Arc;

use brolga_commands::{CommandSpec, builtin_registry};
use brolga_control::{Client, default_endpoint};
use brolga_mcp::{Backend, McpServer, serve_stdio};
use clap::{Parser, Subcommand};
use serde_json::Value;

#[derive(Parser)]
#[command(
    name = "brolga-cli",
    version,
    about = "Run Brolga's commands from a shell, or serve them to an agent over MCP."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List every command, with what it does.
    Commands {
        /// Print each command in full as JSON, schemas included.
        #[arg(long)]
        json: bool,
    },
    /// Run one command and print what it gives back as JSON.
    Run {
        /// The command's id, such as `app.version`.
        id: String,
        /// The command's arguments, as a JSON object.
        #[arg(long, value_name = "JSON", default_value = "{}")]
        json: String,
    },
    /// Serve the commands as MCP tools on standard input and output.
    Mcp {
        /// Only work through the running app, so it shows what the agent does. Fails if the app is not running.
        #[arg(long)]
        connect: bool,
    },
}

/// A failure, as the words to print and the code to exit with.
struct Failure(u8, String);

const FAILED: u8 = 1;
const USAGE: u8 = 2;

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse().command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure(code, message)) => {
            eprintln!("brolga-cli: {message}");
            ExitCode::from(code)
        }
    }
}

async fn run(command: Command) -> Result<(), Failure> {
    match command {
        Command::Commands { json } => {
            let specs = backend(false).await?.specs().await?;
            if json {
                print(&serde_json::to_value(specs).map_err(failed)?)
            } else {
                for spec in specs {
                    println!("{:<24} {}", spec.id, spec.description);
                }
                Ok(())
            }
        }
        Command::Run { id, json } => {
            let args: Value = serde_json::from_str(&json)
                .map_err(|e| Failure(USAGE, format!("--json is not JSON: {e}")))?;
            print(&backend(false).await?.execute(&id, args).await?)
        }
        Command::Mcp { connect } => {
            let server = McpServer::new(match backend(connect).await? {
                Where::Here(registry) => Backend::Headless(registry),
                Where::App(client) => Backend::attached(client),
            });
            serve_stdio(server).await.map_err(failed)
        }
    }
}

/// Where commands run: in the running app if there is one, otherwise here.
enum Where {
    Here(Arc<brolga_commands::Registry>),
    App(Client),
}

async fn backend(app_only: bool) -> Result<Where, Failure> {
    let endpoint = default_endpoint().map_err(failed)?;

    match Client::connect(&endpoint).await {
        Ok(client) => Ok(Where::App(client)),
        Err(_) if app_only => Err(Failure(
            FAILED,
            "the app is not running, so there is nothing to connect to".to_owned(),
        )),
        Err(_) => Ok(Where::Here(Arc::new(builtin_registry().map_err(failed)?))),
    }
}

impl Where {
    async fn specs(self) -> Result<Vec<CommandSpec>, Failure> {
        match self {
            Self::Here(registry) => Ok(registry.specs().cloned().collect()),
            Self::App(mut client) => client.list_commands().await.map_err(failed),
        }
    }

    async fn execute(self, id: &str, args: Value) -> Result<Value, Failure> {
        match self {
            Self::Here(registry) => registry.execute(id, args).await.map_err(failed),
            Self::App(mut client) => client.execute(id, args).await.map_err(failed),
        }
    }
}

fn failed(error: impl std::fmt::Display) -> Failure {
    Failure(FAILED, error.to_string())
}

fn print(value: &Value) -> Result<(), Failure> {
    println!("{}", serde_json::to_string_pretty(value).map_err(failed)?);
    Ok(())
}
