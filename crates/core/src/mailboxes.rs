//! Each account's mail: its database, its connection to Graph and its sync.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use brolga_auth::Accounts;
use brolga_graph::{GraphClient, GraphError, RetryPolicy, TokenSource};
use brolga_store::{Account, AccountDb};
use brolga_sync::MailSync;
use futures::future::BoxFuture;
use tokio::sync::Mutex;

/// One account's mail, opened.
pub(crate) struct Mailbox {
    pub db: Arc<AccountDb>,
    pub graph: Arc<GraphClient>,
    pub sync: MailSync,
    pub calendars: brolga_sync::CalendarSync,
    pub contacts: brolga_sync::ContactSync,
    /// Sends the changes made here to the server.
    pub outbox: Arc<brolga_ops::Outbox>,
    /// One sync of an account at a time: two would fetch the same pages.
    pub syncing: Mutex<()>,
    /// The file that says so to other copies of Brolga on this computer.
    /// None when the mail is kept in memory, which no other copy can see.
    lock_file: Option<PathBuf>,
}

/// The right to sync an account, held against other copies of Brolga (the
/// app, and a command-line or MCP run with no app) until it is dropped.
pub(crate) struct SyncTurn(#[allow(dead_code)] Option<std::fs::File>);

impl Mailbox {
    /// Take the turn to sync, or nothing if another copy has it. Two copies
    /// share one database safely, but syncing it twice at once would fetch
    /// everything twice and each could overwrite the other's place.
    pub fn sync_turn(&self) -> std::io::Result<Option<SyncTurn>> {
        let Some(path) = &self.lock_file else {
            return Ok(Some(SyncTurn(None)));
        };
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;

        match file.try_lock() {
            Ok(()) => Ok(Some(SyncTurn(Some(file)))),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }
}

pub(crate) struct Mailboxes {
    accounts: Arc<Accounts>,
    data_dir: Option<PathBuf>,
    graph_base: Option<String>,
    open: Mutex<HashMap<String, Arc<Mailbox>>>,
}

/// Gets a mailbox's Graph client its access tokens from the account registry.
struct AccountTokens {
    accounts: Arc<Accounts>,
    account_id: String,
}

impl TokenSource for AccountTokens {
    fn token(&self) -> BoxFuture<'_, Result<String, GraphError>> {
        Box::pin(async move {
            // The registry's own words say whether the account needs to sign
            // in again or the sign-in service is out of reach.
            self.accounts
                .access_token(&self.account_id)
                .await
                .map_err(|e| GraphError::Token(e.to_string()))
        })
    }
}

impl Mailboxes {
    pub fn new(
        accounts: Arc<Accounts>,
        data_dir: Option<PathBuf>,
        graph_base: Option<String>,
    ) -> Self {
        Self {
            accounts,
            data_dir,
            graph_base,
            open: Mutex::new(HashMap::new()),
        }
    }

    /// The account asked for, or the only one where none is named.
    pub fn account(&self, id: Option<&str>) -> Result<Account, String> {
        let mut accounts = self.accounts.list().map_err(|e| e.to_string())?;

        match id {
            Some(id) => accounts
                .into_iter()
                .find(|a| a.id == id)
                .ok_or_else(|| format!("There is no account `{id}`.")),
            None if accounts.len() == 1 => Ok(accounts.remove(0)),
            None if accounts.is_empty() => {
                Err("No account is signed in yet. Add one first.".to_owned())
            }
            None => Err(
                "There is more than one account, so say which with account_id (see accounts.list)."
                    .to_owned(),
            ),
        }
    }

    pub fn accounts(&self) -> Result<Vec<Account>, String> {
        self.accounts.list().map_err(|e| e.to_string())
    }

    /// The account's mail, opening its database the first time.
    pub async fn open(&self, account: &Account) -> Result<Arc<Mailbox>, String> {
        let mut open = self.open.lock().await;
        if let Some(mailbox) = open.get(&account.id) {
            return Ok(mailbox.clone());
        }

        let path = self
            .data_dir
            .as_ref()
            .map(|folder| folder.join("accounts").join(file_name(&account.id)));
        let db = Arc::new(
            match &path {
                Some(path) => AccountDb::open(path),
                None => AccountDb::in_memory(),
            }
            .map_err(|e| e.to_string())?,
        );
        let tokens = Arc::new(AccountTokens {
            accounts: self.accounts.clone(),
            account_id: account.id.clone(),
        });
        let graph = Arc::new(
            match &self.graph_base {
                Some(base) => GraphClient::new(base.clone(), tokens, RetryPolicy::impatient()),
                None => GraphClient::production(tokens),
            }
            .map_err(|e| e.to_string())?,
        );
        let mailbox = Arc::new(Mailbox {
            sync: MailSync::new(graph.clone(), db.clone()),
            calendars: brolga_sync::CalendarSync::new(graph.clone(), db.clone()),
            contacts: brolga_sync::ContactSync::new(graph.clone(), db.clone()),
            outbox: Arc::new(brolga_ops::Outbox::new(graph.clone(), db.clone())),
            db,
            graph,
            syncing: Mutex::new(()),
            lock_file: path.map(|path| path.with_extension("sync-lock")),
        });
        open.insert(account.id.clone(), mailbox.clone());

        Ok(mailbox)
    }
}

/// A file name for an account's database. The id is two GUIDs and a dot,
/// but nothing that reaches a path is taken on trust.
fn file_name(account_id: &str) -> String {
    let safe: String = account_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();

    format!("{}.db", safe.trim_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_id_cannot_steer_its_database_out_of_the_folder() {
        assert_eq!(file_name("tenant-1.user-2"), "tenant-1.user-2.db");
        assert_eq!(file_name("../../etc/passwd"), "_.._etc_passwd.db");
        assert_eq!(file_name(".."), ".db");
        assert!(!file_name("a/b\\c").contains(['/', '\\']));
    }
}
