//! The client with no window.
//!
//! [`Core`] opens the data, holds the accounts and builds the one registry
//! of commands that the window, the CLI, the control channel and the MCP
//! server all run. Whatever starts it says where the data lives, where
//! secrets are kept and how a link reaches the browser, so the same core
//! runs in the app, headless in the CLI, and in tests.

mod account_commands;
mod link_commands;
mod mail_commands;
mod mailboxes;
mod remote_images;
mod saving;

use std::path::PathBuf;
use std::sync::Arc;

use brolga_auth::{Accounts, AuthError, Authority, SecretStore, TokenClient};
use brolga_commands::{Registry, RegistryError, builtin_registry};
use brolga_store::{GlobalDb, StoreError};

/// Brolga's own app registration, for organisations that have not made one.
/// There is none yet: registering it with Microsoft is the owner's to do.
/// Until then every sign-in names a client ID of the organisation's own.
pub const DEFAULT_CLIENT_ID: Option<&str> = None;

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
