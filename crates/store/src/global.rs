use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::StoreError;
use crate::migrate::migrate;

/// The schema, one step a version. Steps are only ever added: a released one
/// is never edited, because databases out in the world have already run it.
const STEPS: &[&str] = &[
    "
    CREATE TABLE accounts (
        id TEXT PRIMARY KEY NOT NULL,
        tenant_id TEXT NOT NULL,
        object_id TEXT NOT NULL,
        username TEXT NOT NULL,
        display_name TEXT,
        client_id TEXT NOT NULL,
        tenant TEXT NOT NULL,
        needs_sign_in INTEGER NOT NULL DEFAULT 0,
        added_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
    ) STRICT;
",
    "
    -- Small things remembered on this computer, between accounts.
    CREATE TABLE settings (
        key TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL
    ) STRICT;
",
];

/// A signed-in Microsoft 365 account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Ours, and stable: the organisation's ID and the person's ID in it.
    pub id: String,
    pub tenant_id: String,
    pub object_id: String,
    /// The sign-in name, usually an email address.
    pub username: String,
    pub display_name: Option<String>,
    /// The app registration the account was signed in under.
    pub client_id: String,
    /// The tenant that registration is used with: `organizations`, or one organisation's own.
    pub tenant: String,
    /// The saved sign-in no longer works, and the person must sign in again.
    pub needs_sign_in: bool,
    /// When the account was added, in UTC.
    pub added_at: String,
}

/// The database of things that belong to no one account.
pub struct GlobalDb {
    connection: Mutex<Connection>,
}

impl GlobalDb {
    /// Open the database at this path, creating it and its folder if need be.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }

        Self::prepare(Connection::open(path)?)
    }

    /// A database that lives only in memory. For tests.
    pub fn in_memory() -> Result<Self, StoreError> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(mut connection: Connection) -> Result<Self, StoreError> {
        // Write-ahead logging: readers are not blocked by a writer, and a
        // crash loses at most the transaction that was open.
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        migrate(&mut connection, STEPS)?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Every account, oldest first.
    pub fn accounts(&self) -> Result<Vec<Account>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!("{SELECT} ORDER BY added_at, id"))?;
        let accounts = statement.query_map([], read)?.collect::<Result<_, _>>()?;

        Ok(accounts)
    }

    pub fn account(&self, id: &str) -> Result<Option<Account>, StoreError> {
        Ok(self
            .connection()
            .query_row(&format!("{SELECT} WHERE id = ?1"), [id], read)
            .optional()?)
    }

    /// Add an account, or bring one already here up to date. Signing in again
    /// to an account that needed it lands here, and clears that it did.
    pub fn save_account(&self, account: &Account) -> Result<Account, StoreError> {
        self.connection().execute(
            "INSERT INTO accounts (id, tenant_id, object_id, username, display_name, client_id, tenant, needs_sign_in)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)
             ON CONFLICT (id) DO UPDATE SET
                 username = excluded.username,
                 display_name = excluded.display_name,
                 client_id = excluded.client_id,
                 tenant = excluded.tenant,
                 needs_sign_in = 0",
            params![
                account.id,
                account.tenant_id,
                account.object_id,
                account.username,
                account.display_name,
                account.client_id,
                account.tenant
            ],
        )?;

        // The registration that last worked is offered for the next account,
        // so its client ID is typed once and not for every mailbox. A client
        // ID is not a secret: it names the app, and is in every sign-in link.
        self.connection().execute(
            "INSERT INTO settings (key, value) VALUES ('client_id', ?1)
             ON CONFLICT (key) DO UPDATE SET value = ?1",
            [&account.client_id],
        )?;

        // Just written, so there to read; if it is somehow not, say what was asked for.
        Ok(self
            .account(&account.id)?
            .unwrap_or_else(|| account.clone()))
    }

    /// The client ID the last successful sign-in used, if there has been one.
    pub fn remembered_client_id(&self) -> Result<Option<String>, StoreError> {
        use rusqlite::OptionalExtension;

        Ok(self
            .connection()
            .query_row(
                "SELECT value FROM settings WHERE key = 'client_id'",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn set_needs_sign_in(&self, id: &str, needs_sign_in: bool) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE accounts SET needs_sign_in = ?2 WHERE id = ?1",
            params![id, needs_sign_in],
        )?;

        Ok(())
    }

    /// Remove an account. True if there was one to remove.
    pub fn remove_account(&self, id: &str) -> Result<bool, StoreError> {
        Ok(self
            .connection()
            .execute("DELETE FROM accounts WHERE id = ?1", [id])?
            > 0)
    }
}

const SELECT: &str = "SELECT id, tenant_id, object_id, username, display_name, client_id, tenant, needs_sign_in, added_at
                      FROM accounts";

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        tenant_id: row.get(1)?,
        object_id: row.get(2)?,
        username: row.get(3)?,
        display_name: row.get(4)?,
        client_id: row.get(5)?,
        tenant: row.get(6)?,
        needs_sign_in: row.get(7)?,
        added_at: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, username: &str) -> Account {
        Account {
            id: id.to_owned(),
            tenant_id: "t".to_owned(),
            object_id: id.to_owned(),
            username: username.to_owned(),
            display_name: Some("Robin Example".to_owned()),
            client_id: "client".to_owned(),
            tenant: "organizations".to_owned(),
            needs_sign_in: false,
            added_at: String::new(),
        }
    }

    #[test]
    fn an_account_saved_is_an_account_listed() {
        let db = GlobalDb::in_memory().unwrap();

        let saved = db
            .save_account(&account("t.one", "robin@example.test"))
            .unwrap();

        assert_eq!(db.accounts().unwrap(), std::slice::from_ref(&saved));
        assert_eq!(saved.username, "robin@example.test");
        assert!(!saved.added_at.is_empty());
    }

    #[test]
    fn saving_the_same_account_again_updates_it_and_clears_needs_sign_in() {
        let db = GlobalDb::in_memory().unwrap();
        let first = db
            .save_account(&account("t.one", "robin@example.test"))
            .unwrap();
        db.set_needs_sign_in("t.one", true).unwrap();
        assert!(db.account("t.one").unwrap().unwrap().needs_sign_in);

        let again = db
            .save_account(&account("t.one", "robin.renamed@example.test"))
            .unwrap();

        assert_eq!(db.accounts().unwrap().len(), 1);
        assert_eq!(again.username, "robin.renamed@example.test");
        assert!(!again.needs_sign_in);
        assert_eq!(again.added_at, first.added_at);
    }

    #[test]
    fn several_accounts_are_kept_apart() {
        let db = GlobalDb::in_memory().unwrap();
        db.save_account(&account("t.one", "one@example.test"))
            .unwrap();
        db.save_account(&account("t.two", "two@example.test"))
            .unwrap();

        assert!(db.remove_account("t.one").unwrap());

        let left: Vec<String> = db.accounts().unwrap().into_iter().map(|a| a.id).collect();
        assert_eq!(left, ["t.two"]);
        assert!(!db.remove_account("t.one").unwrap());
    }

    #[test]
    fn accounts_are_still_there_when_the_database_is_opened_again() {
        let folder = std::env::temp_dir().join(format!("brolga-store-test-{}", std::process::id()));
        let path = folder.join("nested").join("global.db");
        {
            let db = GlobalDb::open(&path).unwrap();
            db.save_account(&account("t.one", "robin@example.test"))
                .unwrap();
        }

        let reopened = GlobalDb::open(&path).unwrap();

        assert_eq!(reopened.accounts().unwrap().len(), 1);
        // Windows will not delete a file that is still open.
        drop(reopened);
        let _ = std::fs::remove_dir_all(folder);
    }
}
