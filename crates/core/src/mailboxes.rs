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
    /// One sync of an account at a time: two would fetch the same pages.
    pub syncing: Mutex<()>,
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

        let db = Arc::new(
            match &self.data_dir {
                Some(folder) => {
                    AccountDb::open(&folder.join("accounts").join(file_name(&account.id)))
                }
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
            db,
            graph,
            syncing: Mutex::new(()),
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
