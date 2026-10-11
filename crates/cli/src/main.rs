//! `brolga-cli`: the client's commands from a shell, and the MCP server.
//!
//! When the app is running, commands are passed to it over the control
//! channel, because only one process may own an account's data at a time.
//! When it is not, they run here.

use std::process::ExitCode;
use std::sync::Arc;

use brolga_auth::Authority;
use brolga_commands::CommandSpec;
use brolga_control::{Client, default_endpoint};
use brolga_core::{Core, CoreOptions, DEFAULT_CLIENT_ID};
use brolga_mcp::{Backend, McpServer, serve_stdio};
use brolga_platform::{SystemSecrets, data_dir, open_in_browser};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};

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
    /// Sign in to a Microsoft 365 account and add it.
    SignIn {
        /// Sign in with a code typed in on another device, not this computer's browser.
        #[arg(long)]
        device_code: bool,
        /// The client ID of your organisation's own app registration for Brolga.
        #[arg(long, value_name = "ID")]
        client_id: Option<String>,
        /// Your organisation's tenant ID or domain, with --client-id.
        #[arg(long)]
        tenant: Option<String>,
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
        Command::SignIn {
            device_code,
            client_id,
            tenant,
        } => {
            let mut under = serde_json::Map::new();
            if let Some(client_id) = client_id {
                under.insert("client_id".to_owned(), client_id.into());
            }
            if let Some(tenant) = tenant {
                under.insert("tenant".to_owned(), tenant.into());
            }
            let under = Value::Object(under);

            let added = if device_code {
                let begun = backend(false)
                    .await?
                    .execute("accounts.sign_in_with_code.begin", under)
                    .await?;
                eprintln!("{}", begun["message"].as_str().unwrap_or_default());
                let finish = json!({ "flow_id": begun["flow_id"] });
                backend(false)
                    .await?
                    .execute("accounts.sign_in_with_code.finish", finish)
                    .await?
            } else {
                eprintln!("Finish signing in in your browser.");
                backend(false)
                    .await?
                    .execute("accounts.sign_in", under)
                    .await?
            };
            print(&added)
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
        Err(_) => Ok(Where::Here(core()?.registry())),
    }
}

/// The client, run here with no window: this user's data and secret store.
fn core() -> Result<Core, Failure> {
    let data_dir = data_dir()
        .ok_or_else(|| failed("this computer has no home folder to keep Brolga's data in"))?;

    Core::open(CoreOptions {
        data_dir: Some(data_dir),
        secrets: Arc::new(SystemSecrets),
        open_browser: Arc::new(open_in_browser),
        authority: Authority::default(),
        graph_base: None,
        default_client_id: DEFAULT_CLIENT_ID.map(str::to_owned),
    })
    .map_err(failed)
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
