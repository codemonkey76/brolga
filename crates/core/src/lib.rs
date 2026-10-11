//! The client with no window.
//!
//! [`Core`] opens the data, holds the accounts and builds the one registry
//! of commands that the window, the CLI, the control channel and the MCP
//! server all run. Whatever starts it says where the data lives, where
//! secrets are kept and how a link reaches the browser, so the same core
//! runs in the app, headless in the CLI, and in tests.

mod account_commands;
mod action_commands;
mod draft_commands;
mod link_commands;
mod mail_commands;
mod mailboxes;
mod remote_images;
mod reply_commands;
mod saving;

use std::path::PathBuf;
use std::sync::Arc;

use brolga_auth::{Accounts, AuthError, Authority, SecretStore, TokenClient};
use brolga_commands::{Registry, RegistryError, builtin_registry};
use brolga_store::{GlobalDb, StoreError};

pub use saving::safe_name as safe_file_name;

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
}

pub struct Core {
    accounts: Arc<Accounts>,
    registry: Arc<Registry>,
}

impl Core {
    pub fn open(options: CoreOptions) -> Result<Self, CoreError> {
        let db = Arc::new(match &options.data_dir {
            Some(folder) => GlobalDb::open(&folder.join("global.db"))?,
            None => GlobalDb::in_memory()?,
        });
        let accounts = Arc::new(Accounts::new(
            db,
            options.secrets,
            TokenClient::new(options.authority)?,
        ));

        let mailboxes = Arc::new(mailboxes::Mailboxes::new(
            accounts.clone(),
            options.data_dir.clone(),
            options.graph_base.clone(),
        ));

        let mut registry = builtin_registry()?;
        let images = Arc::new(remote_images::RemoteImages::new(
            options.fetch_images_from_private_hosts,
        ));
        action_commands::register(&mut registry, mailboxes.clone())?;
        draft_commands::register(&mut registry, mailboxes.clone())?;
        reply_commands::register(&mut registry, mailboxes.clone())?;
        mail_commands::register(
            &mut registry,
            mail_commands::Context {
                mailboxes,
                images,
                downloads: options.downloads_dir.clone(),
            },
        )?;
        link_commands::register(&mut registry, options.open_browser.clone())?;
        account_commands::register(
            &mut registry,
            account_commands::Context {
                accounts: accounts.clone(),
                open_browser: options.open_browser,
                default_client_id: options.default_client_id,
                device_sign_ins: Default::default(),
            },
        )?;

        Ok(Self {
            accounts,
            registry: Arc::new(registry),
        })
    }

    /// Every command the client has.
    pub fn registry(&self) -> Arc<Registry> {
        self.registry.clone()
    }

    pub fn accounts(&self) -> Arc<Accounts> {
        self.accounts.clone()
    }
}
