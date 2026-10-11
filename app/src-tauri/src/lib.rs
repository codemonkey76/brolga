//! The window.
//!
//! It holds the command registry, runs commands for the frontend, and listens
//! on the control channel so the CLI and agents can run the same commands in
//! this process while it is open.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use brolga_auth::Authority;
use brolga_commands::{CommandError, Registry};
use brolga_control::{Client, Server, ServerError, default_endpoint};
use brolga_core::{Core, CoreOptions, DEFAULT_CLIENT_ID, safe_file_name};
use brolga_platform::{SystemSecrets, data_dir, downloads_dir, open_in_browser};
use serde::Serialize;
use serde_json::Value;
use tauri::{Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;
use tokio::sync::mpsc;

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

/// Ask which files to attach, with the system's own dialog. Gives back
/// their paths, or none if the dialog was dismissed. Choosing is all it
/// does: the attaching is a command.
#[tauri::command]
async fn choose_files(app: tauri::AppHandle) -> Vec<String> {
    let (chosen, choice) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_files(move |paths| {
        let _ = chosen.send(paths);
    });

    choice
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

/// Show a notification on the desktop. The words are a reminder's: an
/// event's subject and when it starts. Showing is all this does; if the
/// desktop will not show it, the reminder is still in the window.
#[tauri::command]
fn notify(app: tauri::AppHandle, title: String, body: String) {
    // Kept short: a subject is written by whoever made the event.
    let short = |text: String, most: usize| text.chars().take(most).collect::<String>();
    let _ = app
        .notification()
        .builder()
        .title(short(title, 120))
        .body(short(body, 240))
        .show();
}

/// Messages the window has been asked to start, by a `mailto:` link, each
/// waiting for the window to take it.
struct Asked(tokio::sync::Mutex<mpsc::UnboundedReceiver<Value>>);

/// The next message the window has been asked to start: who to, the subject
/// and the words, as `app.window.show` read them from a link. It answers
/// when there is one, however long that is.
#[tauri::command]
async fn next_request(asked: State<'_, Asked>) -> Result<Option<Value>, ()> {
    Ok(asked.0.lock().await.recv().await)
}

/// Whether closing the window leaves Brolga running. True only when the
/// person chose it and there is a tray icon to bring it back, or quit, by.
#[derive(Default)]
struct Stays {
    chosen: AtomicBool,
    tray: AtomicBool,
}

impl Stays {
    fn on_close(&self) -> bool {
        self.chosen.load(Ordering::Relaxed) && self.tray.load(Ordering::Relaxed)
    }
}

/// Act on what `app.desktop.set` recorded: whether closing leaves Brolga
/// running, and whether it starts when the person signs in. Gives back
/// whether there is a tray icon, without which it cannot stay.
#[tauri::command]
async fn apply_desktop(
    app: tauri::AppHandle,
    registry: State<'_, Arc<Registry>>,
    stays: State<'_, Arc<Stays>>,
) -> Result<bool, Failure> {
    let chosen = registry
        .execute("app.desktop.get", serde_json::json!({}))
        .await?;
    stays
        .chosen
        .store(chosen["keep_running"] == true, Ordering::Relaxed);
    let starter = app.autolaunch();
    let _ = if chosen["start_at_login"] == true {
        starter.enable()
    } else {
        starter.disable()
    };

    Ok(stays.tray.load(Ordering::Relaxed))
}

/// An icon in the system tray, to bring the window back by and to quit
/// from. A desktop with no tray has none, and is told so by the error.
fn tray(app: &tauri::AppHandle, registry: Arc<Registry>) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};

    let menu = MenuBuilder::new(app)
        .item(&MenuItemBuilder::with_id("open", "Open Brolga").build(app)?)
        .item(&MenuItemBuilder::with_id("write", "New message").build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id("quit", "Quit").build(app)?)
        .build()?;
    let mut icon = tauri::tray::TrayIconBuilder::with_id("main")
        .tooltip("Brolga")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "quit" => app.exit(0),
            "write" => {
                let registry = registry.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = registry
                        .execute("app.window.show", showing(Some("mailto:")))
                        .await;
                });
            }
            _ => bring_forward(app),
        });
    if let Some(picture) = app.default_window_icon() {
        icon = icon.icon(picture.clone());
    }
    icon.build(app)?;

    Ok(())
}

fn bring_forward(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// What `app.window.show` is given: nothing, or the link this was started with.
fn showing(link: Option<&str>) -> Value {
    match link {
        Some(link) => serde_json::json!({ "mailto": link }),
        None => serde_json::json!({}),
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Started by the computer at sign-in, not by a person: no window is shown.
    let background = std::env::args().skip(1).any(|arg| arg == "--background");
    // A mailto: link clicked somewhere else arrives as the one argument.
    let link = std::env::args().skip(1).find(|arg| {
        arg.get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case("mailto:"))
    });

    // Opening the channel is how a second copy finds out it is one, so it is
    // done first, before any data is opened or there is a window to show.
    let endpoint = default_endpoint()?;
    let server = match tauri::async_runtime::block_on(Server::bind(&endpoint)) {
        Ok(server) => server,
        // The copy already running is brought forward, and given the link.
        Err(ServerError::AlreadyRunning) => {
            return tauri::async_runtime::block_on(async {
                let mut running = Client::connect(&endpoint).await?;
                running
                    .execute("app.window.show", showing(link.as_deref()))
                    .await?;
                Ok(())
            });
        }
        Err(other) => return Err(other.into()),
    };

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

    let (ask, asked) = mpsc::unbounded_channel();
    let window: Arc<OnceLock<tauri::AppHandle>> = Arc::new(OnceLock::new());
    let shown = window.clone();
    core.on_show_window(Arc::new(move |writing| {
        if let Some(writing) = writing {
            let _ = ask.send(writing);
        }
        if let Some(app) = shown.get() {
            bring_forward(app);
        }
    }));
    let opening = registry.clone();
    let for_tray = registry.clone();
    let at_start = registry.clone();
    let stays = Arc::new(Stays::default());
    let (staying, closing) = (stays.clone(), stays.clone());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .manage(stays)
        // Closing the window hides it where the person chose that Brolga
        // stays; mail goes on being fetched and reminders given.
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && closing.on_close()
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .manage(registry)
        .manage(Asked(tokio::sync::Mutex::new(asked)))
        .setup(move |app| {
            let _ = window.set(app.handle().clone());
            let has_tray = tray(app.handle(), for_tray).is_ok();
            staying.tray.store(has_tray, Ordering::Relaxed);
            let chosen = tauri::async_runtime::block_on(
                at_start.execute("app.desktop.get", serde_json::json!({})),
            )
            .unwrap_or_default();
            staying
                .chosen
                .store(chosen["keep_running"] == true, Ordering::Relaxed);
            // With no tray to come back by, a window is always shown.
            if !(background && staying.on_close()) {
                bring_forward(app.handle());
            }
            tauri::async_runtime::spawn(server.run(control));
            if link.is_some() {
                tauri::async_runtime::spawn(async move {
                    let _ = opening
                        .execute("app.window.show", showing(link.as_deref()))
                        .await;
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            execute_command,
            choose_save_path,
            choose_files,
            notify,
            next_request,
            apply_desktop
        ])
        .build(tauri::generate_context!())?
        .run(|_app, _event| {
            // macOS: the icon in the Dock brings a hidden window back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                bring_forward(_app);
            }
            // macOS hands a clicked link to the running app, not to a new one.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = _event {
                let registry = _app.state::<Arc<Registry>>().inner().clone();
                for url in urls {
                    let registry = registry.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = registry
                            .execute("app.window.show", showing(Some(url.as_str())))
                            .await;
                    });
                }
            }
        });

    Ok(())
}
