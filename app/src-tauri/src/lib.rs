//! The window.
//!
//! It holds the command registry, runs commands for the frontend, and listens
//! on the control channel so the CLI and agents can run the same commands in
//! this process while it is open.

use std::sync::Arc;

use brolga_auth::Authority;
use brolga_commands::{CommandError, Registry};
use brolga_control::{Server, default_endpoint};
use brolga_core::{Core, CoreOptions, DEFAULT_CLIENT_ID, safe_file_name};
use brolga_platform::{SystemSecrets, data_dir, downloads_dir, open_in_browser};
use serde::Serialize;
use serde_json::Value;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

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

/// Ask where to save a file, with the system's own dialog. Gives back the
/// place chosen, or nothing if the dialog was dismissed. Choosing is all it
/// does: the saving is a command, like everything else.
///
/// The name offered is the sender's, cut down to a plain file name first.
#[tauri::command]
async fn choose_save_path(app: tauri::AppHandle, suggested_name: String) -> Option<String> {
    let (chosen, choice) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .set_file_name(safe_file_name(&suggested_name));
    if let Some(downloads) = downloads_dir() {
        dialog = dialog.set_directory(downloads);
    }
    dialog.save_file(move |path| {
        let _ = chosen.send(path);
    });

    let path = choice.await.ok()??.into_path().ok()?;

    Some(path.to_string_lossy().into_owned())
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Opening the channel is how a second copy finds out it is one, so it is
    // done first, before any data is opened or there is a window to show.
    let endpoint = default_endpoint()?;
    let server = tauri::async_runtime::block_on(Server::bind(&endpoint))?;

    let core = Core::open(CoreOptions {
        data_dir: Some(
            data_dir().ok_or("this computer has no home folder to keep Brolga's data in")?,
        ),
        downloads_dir: downloads_dir(),
        secrets: Arc::new(SystemSecrets),
        open_browser: Arc::new(open_in_browser),
        authority: Authority::default(),
        graph_base: None,
        default_client_id: DEFAULT_CLIENT_ID.map(str::to_owned),
        fetch_images_from_private_hosts: false,
    })?;
    let registry = core.registry();
    let control = registry.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(registry)
        .setup(move |_app| {
            tauri::async_runtime::spawn(server.run(control));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![execute_command, choose_save_path])
        .run(tauri::generate_context!())?;

    Ok(())
}
