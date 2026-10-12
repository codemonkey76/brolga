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
use brolga_core::{Core, CoreOptions, DEFAULT_CLIENT_ID, keep_synced};
use brolga_mcp::{Access, Backend, McpServer, Policy, serve_stdio};
use brolga_platform::{SystemSecrets, data_dir, downloads_dir, open_in_browser};
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
    ///
    /// This runs the command as you, with none of the limits `mcp` puts on an agent: no access
    /// level, no asking first, no record. Answering an agent's question is the one thing it
    /// cannot do: that is done in the app's window.
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
    /// For developers: ask Microsoft Graph read-only questions as a signed-in account, and print how it answers.
    ///
    /// With no path, it asks each way there might be of learning that a message was answered or
    /// forwarded, and says which the server takes. Only message ids are asked for. Whatever is asked,
    /// no mail is printed: an answer is summed up as how many things it gave, or why it was refused.
    Probe {
        /// What follows `/me` in the request, such as `/messages?$top=1`. Put it in single quotes.
        /// It stays under `/me`: a path with `..` in it is refused.
        path: Option<String>,
        /// Which account, by its id. May be left out when there is only one.
        #[arg(long)]
        account: Option<String>,
    },
    /// Serve the commands as MCP tools on standard input and output.
    ///
    /// The access level and the asking bind an agent whose only way to Brolga is this server. An
    /// agent that can also run programs as you can run `brolga-cli run` itself, which has no limits.
    Mcp {
        /// Only work through the running app, so it shows what the agent does. Fails if the app is not running.
        #[arg(long)]
        connect: bool,
        /// How far the agent is trusted: `read` (the default) lets it read mail and nothing else;
        /// `draft` lets it also write drafts, which reach nobody until sent; `full` lets it change the mailbox.
        #[arg(long, default_value = "read")]
        access: AgentAccess,
        /// Let the agent run commands that reach other people or cannot be undone, such as sending,
        /// without asking you. Without this, and with `--access full`, each one is put to you in the
        /// app's window and runs only if you allow it; with no window open the answer is no.
        #[arg(long)]
        allow_consequential: bool,
        /// The most messages the agent may send in any hour.
        #[arg(long, default_value_t = 20)]
        max_sends_per_hour: u32,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum AgentAccess {
    Read,
    Draft,
    Full,
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
        Command::Probe { path, account } => {
            let core = core()?;
            let asked: Vec<(String, String)> = match path {
                Some(path) => vec![("asked for".to_owned(), path)],
                None => last_action_probes(),
            };
            for (what, path) in asked {
                println!("{what}\n  GET /me{path}");
                match core.probe(account.as_deref(), &path).await {
                    Ok((status, said)) => println!("  {status}: {}\n", summed_up(status, &said)),
                    Err(error) => println!("  not asked: {error}\n"),
                }
            }
            Ok(())
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
        Command::Mcp {
            connect,
            access,
            allow_consequential,
            max_sends_per_hour,
        } => {
            let policy = Policy {
                access: match access {
                    AgentAccess::Read => Access::Read,
                    AgentAccess::Draft => Access::Draft,
                    AgentAccess::Full => Access::Full,
                },
                allow_consequential,
                max_sends_per_hour,
            };
            let server = McpServer::with_policy(
                match backend(connect).await? {
                    Where::Here(registry) => {
                        // With no app running there is no window keeping the
                        // mail current, so it is done here for as long as the
                        // server runs.
                        tokio::spawn(keep_synced(registry.clone(), SYNC_EVERY));
                        Backend::Headless(registry)
                    }
                    Where::App(client) => Backend::attached(client),
                },
                policy,
            );
            serve_stdio(server).await.map_err(failed)
        }
    }
}

/// Each way there might be of learning from Graph that a message was answered
/// or forwarded: the property `Integer 0x1081` ([MS-OXOMSG],
/// PidTagLastVerbExecuted), which is 102 for a reply, 103 for a reply to all
/// and 104 for a forward. Which of these the server takes is what is being
/// found out. Only ids are asked for.
fn last_action_probes() -> Vec<(String, String)> {
    const EXPAND: &str = "$expand=singleValueExtendedProperties($filter=id eq 'Integer 0x1081')";
    let has = |test: &str| {
        format!(
            "singleValueExtendedProperties/Any(ep: ep/id eq 'Integer 0x1081' and cast(ep/value, Edm.Int32) {test})"
        )
    };
    let all =
        |what: &str, query: String| (what.to_owned(), format!("/messages?$select=id&{query}"));

    vec![
        all(
            "1. Any messages, with the property given beside each",
            EXPAND.to_owned(),
        ),
        all(
            "2. Only messages replied to",
            format!("$filter={}", has("eq 102")),
        ),
        all(
            "3. Only messages forwarded",
            format!("$filter={}", has("eq 104")),
        ),
        all(
            "4. Only messages answered or forwarded, of any kind",
            format!("$filter={}", has("ge 102")),
        ),
        all(
            "5. Those, with the property given beside each",
            format!("$filter={}&{EXPAND}", has("ge 102")),
        ),
        all(
            "6. Those changed since the start of the month",
            format!(
                "$filter=lastModifiedDateTime ge 2026-10-01T00:00:00Z and {}",
                has("ge 102")
            ),
        ),
        (
            "7. Those in the inbox alone, with the property".to_owned(),
            format!(
                "/mailFolders/inbox/messages?$select=id&$filter={}&{EXPAND}",
                has("ge 102")
            ),
        ),
        all(
            "10. What Brolga asks: those changed since the start of the month, with the property, two hundred at a time",
            format!(
                "$top=200&$filter=lastModifiedDateTime ge 2026-10-01T00:00:00Z and {}&{EXPAND}",
                has("ge 102")
            ),
        ),
        all(
            "11. And the same of the whole mailbox, as it is asked the first time",
            format!("$top=200&$filter={}&{EXPAND}", has("ge 102")),
        ),
        (
            "8. A sync of the inbox with the property and nothing else chosen".to_owned(),
            format!("/mailFolders/inbox/messages/delta?{EXPAND}"),
        ),
        (
            "9. A sync of the inbox with the property and ids chosen (what 0.1.5 asked, cut down)"
                .to_owned(),
            format!("/mailFolders/inbox/messages/delta?$select=id&{EXPAND}"),
        ),
    ]
}

/// An answer from Graph in a line: how many things it gave and what the
/// property was on them, or why it refused. No mail is repeated.
fn summed_up(status: u16, said: &str) -> String {
    let Ok(answer) = serde_json::from_str::<Value>(said) else {
        // Not repeated: it could be a message, or a file attached to one.
        return format!("an answer that is not JSON, of {} bytes", said.len());
    };
    if !(200..300).contains(&status) {
        let error = &answer["error"];
        return format!(
            "refused ({}): {}",
            error["code"].as_str().unwrap_or("no code"),
            error["message"].as_str().unwrap_or("no reason given")
        );
    }
    let Some(given) = answer["value"].as_array() else {
        return "taken, and one thing given".to_owned();
    };
    let values: Vec<String> = given
        .iter()
        .flat_map(|each| {
            each["singleValueExtendedProperties"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .map(|property| property["value"].to_string())
        .collect();
    let more = if answer["@odata.nextLink"].is_string() {
        ", and more to come"
    } else {
        ""
    };

    format!(
        "taken; {} given{more}; the property on {} of them{}",
        given.len(),
        values.len(),
        if values.is_empty() {
            String::new()
        } else {
            format!(": {}", values.join(", "))
        }
    )
}

/// How often mail is brought up to date when there is no window to do it.
const SYNC_EVERY: std::time::Duration = std::time::Duration::from_secs(120);

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
        downloads_dir: downloads_dir(),
        secrets: Arc::new(SystemSecrets),
        open_browser: Arc::new(open_in_browser),
        authority: Authority::default(),
        graph_base: None,
        default_client_id: DEFAULT_CLIENT_ID.map(str::to_owned),
        fetch_images_from_private_hosts: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_to_a_probe_is_summed_up_and_never_repeated() {
        let mime = "Subject: Secret plans\r\n\r\nMeet at dawn.";
        let summed = summed_up(200, mime);
        assert_eq!(summed, "an answer that is not JSON, of 38 bytes");

        let listed = r#"{"value":[{"id":"a","subject":"Secret plans"}]}"#;
        assert!(!summed_up(200, listed).contains("Secret"));
    }
}
