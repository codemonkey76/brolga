//! The client with no window.
//!
//! [`Core`] opens the data, holds the accounts and builds the one registry
//! of commands that the window, the CLI, the control channel and the MCP
//! server all run. Whatever starts it says where the data lives, where
//! secrets are kept and how a link reaches the browser, so the same core
//! runs in the app, headless in the CLI, and in tests.

mod account_commands;

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
    pub secrets: Arc<dyn SecretStore>,
    pub open_browser: OpenBrowser,
    pub authority: Authority,
    pub default_client_id: Option<String>,
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

        let mut registry = builtin_registry()?;
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
