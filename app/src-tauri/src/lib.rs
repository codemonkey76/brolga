//! The window.
//!
//! It holds the command registry, runs commands for the frontend, and listens
//! on the control channel so the CLI and agents can run the same commands in
//! this process while it is open.

use std::sync::Arc;

use brolga_commands::{CommandError, Registry, builtin_registry};
use brolga_control::{Server, default_endpoint};
use serde::Serialize;
use serde_json::Value;
use tauri::State;

/// A command that did not run, as the frontend receives it.
#[derive(Serialize)]
struct Failure {
    code: &'static str,
    message: String,
}

impl From<CommandError> for Failure {
    fn from(error: CommandError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
        }
    }
}

/// The frontend's one way in: run a command by id.
#[tauri::command]
async fn execute_command(
    registry: State<'_, Arc<Registry>>,
    command: String,
    args: Option<Value>,
) -> Result<Value, Failure> {
    let args = args.unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    Ok(registry.execute(&command, args).await?)
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let registry = Arc::new(builtin_registry()?);

    // Opening the channel is also how a second copy finds out it is one, so
    // it is done before there is a window to show.
    let endpoint = default_endpoint()?;
    let server = tauri::async_runtime::block_on(Server::bind(&endpoint, registry.clone()))?;

    tauri::Builder::default()
        .manage(registry)
        .setup(move |_app| {
            tauri::async_runtime::spawn(server.run());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![execute_command])
        .run(tauri::generate_context!())?;

    Ok(())
}
