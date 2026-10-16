//! The client with no window.
//!
//! [`Core`] opens the data, holds the accounts and builds the one registry
//! of commands that the window, the CLI, the control channel and the MCP
//! server all run. Whatever starts it says where the data lives, where
//! secrets are kept and how a link reaches the browser, so the same core
//! runs in the app, headless in the CLI, and in tests.

mod account_commands;
mod action_commands;
mod agent_commands;
mod archive_commands;
mod calendar_commands;
mod contact_commands;
mod deciding;
mod device;
mod draft_commands;
mod holiday_commands;
mod instruction_commands;
mod instructions;
mod link_commands;
mod mail_commands;
mod mailboxes;
mod processing;
mod remote_images;
mod reply_commands;
mod rule_commands;
mod runs;
mod saving;
mod shared_commands;
mod sharing_commands;
mod task_commands;
mod tool_commands;
mod window_commands;

use std::path::PathBuf;
use std::sync::Arc;

use brolga_auth::{Accounts, AuthError, Authority, SecretStore, TokenClient};
use brolga_commands::{Registry, RegistryError, builtin_registry};
use brolga_store::{GlobalDb, StoreError};

pub use saving::safe_name as safe_file_name;
pub use window_commands::ShowWindow;

/// Brolga's own app registration with Microsoft: multi-tenant, a public
/// client, redirecting to `http://localhost`. A client ID is not a secret;
/// it names the app and is in every sign-in link.
///
/// It is the owner's development registration, and its publisher is not
/// verified. An organisation that allows only apps it has approved enters
/// the client ID of its own registration instead, under Advanced.
pub const DEFAULT_CLIENT_ID: Option<&str> = Some("3b0d6477-515b-429a-96b2-f8759528a5cc");

/// Hands a link to the person's browser.
pub type OpenBrowser = Arc<dyn Fn(&str) -> std::io::Result<()> + Send + Sync>;

pub struct CoreOptions {
    /// The folder the databases live in. `None` keeps everything in memory, for tests.
    pub data_dir: Option<PathBuf>,
    /// Where a saved attachment goes. `None` and nothing can be saved.
    pub downloads_dir: Option<PathBuf>,
    pub secrets: Arc<dyn SecretStore>,
    pub open_browser: OpenBrowser,
    pub authority: Authority,
    /// Where Microsoft Graph is. `None` is the real one; tests name a stand-in.
    pub graph_base: Option<String>,
    pub default_client_id: Option<String>,
    /// Let a message's remote images be fetched from this computer or a
    /// private network. Always false outside tests.
    pub fetch_images_from_private_hosts: bool,
    /// The computer's own name, which this copy of Brolga is first named
    /// after, where it is known.
    pub host_name: Option<String>,
    /// Where Anthropic's API is. `None` is the real one; tests name a stand-in.
    pub anthropic_base: Option<String>,
}

/// Keep every account's mail up to date for as long as this is polled: a
/// sync now, and another each time `every` has passed. A sync that fails is
/// tried again at the next turn, and says why in `app.sync.status`.
///
/// The window does this for itself while it is open. This is for the client
/// running with no window, where nothing else would.
pub async fn keep_synced(registry: Arc<Registry>, every: std::time::Duration) {
    let mut turn = tokio::time::interval(every);
    // After a long sync, or the computer waking, one sync and not a queue of them.
    turn.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        turn.tick().await;
        let _ = registry.execute("mail.sync", serde_json::json!({})).await;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("Brolga's commands could not be set up: {0}")]
    Registry(#[from] RegistryError),
    #[error("This device could not be given its id: {0}")]
    Device(String),
}

pub struct Core {
    accounts: Arc<Accounts>,
    mailboxes: Arc<mailboxes::Mailboxes>,
    registry: Arc<Registry>,
    window: Arc<std::sync::OnceLock<ShowWindow>>,
}

impl Core {
    pub fn open(options: CoreOptions) -> Result<Self, CoreError> {
        let db = Arc::new(match &options.data_dir {
            Some(folder) => GlobalDb::open(&folder.join("global.db"))?,
            None => GlobalDb::in_memory()?,
        });
        let global = db.clone();
        device::settle(&global, options.host_name.as_deref()).map_err(CoreError::Device)?;
        let accounts = Arc::new(Accounts::new(
            db,
            options.secrets.clone(),
            TokenClient::new(options.authority)?,
        ));

        let mailboxes = Arc::new(mailboxes::Mailboxes::new(
            global.clone(),
            accounts.clone(),
            options.data_dir.clone(),
            options.graph_base.clone(),
            mailboxes::Ai {
                secrets: options.secrets.clone(),
                anthropic_base: options
                    .anthropic_base
                    .clone()
                    .unwrap_or_else(|| brolga_ai::ANTHROPIC.to_owned()),
                http: brolga_ai::client().ok(),
            },
        ));

        let mut registry = builtin_registry()?;
        let images = Arc::new(remote_images::RemoteImages::new(
            options.fetch_images_from_private_hosts,
        ));
        agent_commands::register(&mut registry, global.clone())?;
        action_commands::register(&mut registry, mailboxes.clone())?;
        draft_commands::register(&mut registry, mailboxes.clone())?;
        calendar_commands::register(&mut registry, mailboxes.clone())?;
        holiday_commands::register(&mut registry, mailboxes.clone())?;
        contact_commands::register(
            &mut registry,
            mailboxes.clone(),
            options.downloads_dir.clone(),
        )?;
        shared_commands::register(&mut registry, mailboxes.clone())?;
        rule_commands::register(&mut registry, mailboxes.clone())?;
        instruction_commands::register(&mut registry, mailboxes.clone())?;
        task_commands::register(&mut registry, mailboxes.clone())?;
        tool_commands::register(
            &mut registry,
            mailboxes.clone(),
            options.open_browser.clone(),
        )?;
        sharing_commands::register(&mut registry, mailboxes.clone())?;
        archive_commands::register(&mut registry, mailboxes.clone())?;
        reply_commands::register(&mut registry, mailboxes.clone())?;
        let probed = mailboxes.clone();
        mail_commands::register(
            &mut registry,
            mail_commands::Context {
                mailboxes,
                images,
                downloads: options.downloads_dir.clone(),
            },
        )?;
        link_commands::register(&mut registry, options.open_browser.clone())?;
        let window = Arc::new(std::sync::OnceLock::new());
        window_commands::register(&mut registry, window.clone(), global)?;
        account_commands::register(
            &mut registry,
            account_commands::Context {
                accounts: accounts.clone(),
                mailboxes: probed.clone(),
                open_browser: options.open_browser,
                default_client_id: options.default_client_id,
                device_sign_ins: Default::default(),
            },
        )?;

        let registry = Arc::new(registry);
        probed.set_registry(&registry);

        Ok(Self {
            accounts,
            mailboxes: probed,
            registry,
            window,
        })
    }

    /// Ask Microsoft Graph one read-only question as an account, and give
    /// back the status and the words of its answer, taken or refused.
    ///
    /// This is a developer's tool for learning what the server makes of a
    /// request before anything is built on it. It is not a command: nothing
    /// that runs commands, an agent least of all, can reach it.
    pub async fn probe(
        &self,
        account_id: Option<&str>,
        path: &str,
    ) -> Result<(u16, String), String> {
        let account = self.mailboxes.account(account_id)?;
        let mailbox = self.mailboxes.open(&account).await?;

        mailbox.graph.probe(path).await.map_err(|e| e.to_string())
    }

    /// Every command the client has.
    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }

    /// Say how the window is brought forward, for `app.window.show`. Said
    /// once, by whatever has a window; a second saying is ignored.
    pub fn on_show_window(&self, show: ShowWindow) {
        let _ = self.window.set(show);
    }

    pub fn accounts(&self) -> Arc<Accounts> {
        self.accounts.clone()
    }
}
