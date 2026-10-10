use std::sync::Arc;

use brolga_commands::{CommandSpec, Registry};
use brolga_control::{Client, ClientError};
use serde_json::Value;
use tokio::sync::Mutex;

/// Where the server's commands are run.
pub enum Backend {
    /// In this process. Used when no client is running.
    Headless(Arc<Registry>),
    /// In the running client, so the person using it sees what the agent does.
    Attached(Mutex<Client>),
}

/// A command that did not run, in words fit to show an agent.
pub struct Refusal(pub String);

impl Backend {
    pub fn attached(client: Client) -> Self {
        Self::Attached(Mutex::new(client))
    }

    pub(crate) async fn specs(&self) -> Result<Vec<CommandSpec>, Refusal> {
        match self {
            Self::Headless(registry) => Ok(registry.specs().cloned().collect()),
            Self::Attached(client) => client.lock().await.list_commands().await.map_err(refusal),
        }
    }

    pub(crate) async fn execute(&self, command: &str, args: Value) -> Result<Value, Refusal> {
        match self {
            Self::Headless(registry) => registry
                .execute(command, args)
                .await
                .map_err(|e| Refusal(e.to_string())),
            Self::Attached(client) => client
                .lock()
                .await
                .execute(command, args)
                .await
                .map_err(refusal),
        }
    }
}

fn refusal(error: ClientError) -> Refusal {
    Refusal(error.to_string())
}
