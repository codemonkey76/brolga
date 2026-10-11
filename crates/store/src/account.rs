//! One account's mail.
//!
//! The database is the single source of truth for everything the window
//! shows. Sync writes to it a page at a time, and each page is written
//! together with the place the sync has reached, in one transaction: after a
//! crash the database holds whole pages only, and knows where to carry on.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::StoreError;
use crate::migrate::migrate;

const STEPS: &[&str] = &["
    CREATE TABLE folders (
        id TEXT PRIMARY KEY NOT NULL,
        parent_id TEXT,
        name TEXT NOT NULL DEFAULT '',
        -- inbox, drafts, sentitems and so on, for the folders every mailbox has.
        well_known TEXT,
        unread INTEGER NOT NULL DEFAULT 0,
        total INTEGER NOT NULL DEFAULT 0,
        hidden INTEGER NOT NULL DEFAULT 0,
        generation INTEGER NOT NULL DEFAULT 0
    ) STRICT;

    CREATE TABLE messages (
        id TEXT PRIMARY KEY NOT NULL,
        folder_id TEXT NOT NULL,
        conversation_id TEXT,
        subject TEXT NOT NULL DEFAULT '',
        preview TEXT NOT NULL DEFAULT '',
        from_name TEXT NOT NULL DEFAULT '',
        from_address TEXT NOT NULL DEFAULT '',
        -- JSON arrays of {name, address}.
        to_recipients TEXT NOT NULL DEFAULT '[]',
        cc_recipients TEXT NOT NULL DEFAULT '[]',
        received_at TEXT NOT NULL DEFAULT '',
        sent_at TEXT,
        is_read INTEGER NOT NULL DEFAULT 0,
        is_draft INTEGER NOT NULL DEFAULT 0,
        has_attachments INTEGER NOT NULL DEFAULT 0,
        importance TEXT NOT NULL DEFAULT 'normal',
        flag_status TEXT NOT NULL DEFAULT 'notFlagged',
        categories TEXT NOT NULL DEFAULT '[]',
        inference TEXT NOT NULL DEFAULT 'focused',
        generation INTEGER NOT NULL DEFAULT 0
    ) STRICT;

    -- The message list: one folder, newest first.
    CREATE INDEX messages_by_folder ON messages (folder_id, received_at DESC, id);

    CREATE TABLE bodies (
        message_id TEXT PRIMARY KEY NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
        content_type TEXT NOT NULL,
        content TEXT NOT NULL,
        -- The body as plain words, for search.
        text TEXT NOT NULL DEFAULT '',
        fetched_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
    ) STRICT;

    -- Where each delta sync has reached. `link` is the URL to ask next; a
    -- round with `reconciling` set is a full pass, after which anything in
    -- the resource not seen during it (an older generation) is gone.
    CREATE TABLE sync_state (
        resource TEXT PRIMARY KEY NOT NULL,
        link TEXT,
        complete INTEGER NOT NULL DEFAULT 0,
        generation INTEGER NOT NULL DEFAULT 1,
        reconciling INTEGER NOT NULL DEFAULT 1,
        synced_at TEXT
    ) STRICT;

    -- Full-text search. Its rowid is the message's rowid.
    CREATE VIRTUAL TABLE messages_fts USING fts5 (
        subject, sender, recipients, body,
        tokenize = 'unicode61 remove_diacritics 2'
    );
"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub well_known: Option<String>,
    pub unread: i64,
    pub total: i64,
    pub hidden: bool,
}

/// What a sync learned about a folder. `None` leaves what is stored alone,
/// because a delta entry carries only what changed.
#[derive(Debug, Clone, Default)]
pub struct FolderChange {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: Option<String>,
    pub unread: Option<i64>,
    pub total: Option<i64>,
    pub hidden: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRow {
    pub id: String,
    pub folder_id: String,
    pub conversation_id: Option<String>,
    pub subject: String,
    pub preview: String,
    pub from_name: String,
    pub from_address: String,
    /// JSON: an array of `{name, address}`.
    pub to_recipients: String,
    pub cc_recipients: String,
    pub received_at: String,
    pub sent_at: Option<String>,
    pub is_read: bool,
    pub is_draft: bool,
    pub has_attachments: bool,
    pub importance: String,
    pub flag_status: String,
    /// JSON: an array of strings.
    pub categories: String,
    pub inference: String,
}

/// What a sync learned about a message. As with a folder, `None` leaves
/// what is stored alone.
#[derive(Debug, Clone, Default)]
pub struct MessageChange {
    pub id: String,
    pub conversation_id: Option<String>,
    pub subject: Option<String>,
    pub preview: Option<String>,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub to_recipients: Option<String>,
    pub cc_recipients: Option<String>,
    pub received_at: Option<String>,
    pub sent_at: Option<String>,
    pub is_read: Option<bool>,
    pub is_draft: Option<bool>,
    pub has_attachments: Option<bool>,
    pub importance: Option<String>,
    pub flag_status: Option<String>,
    pub categories: Option<String>,
    pub inference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    /// `text` or `html`.
    pub content_type: String,
    pub content: String,
}

/// Where a resource's sync stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncState {
    /// The URL to ask next. None before the first page.
    pub link: Option<String>,
    /// True once a round has finished and `link` asks for changes since.
    pub complete: bool,
    pub generation: i64,
    /// True during a full pass: the first sync, or one after the server
    /// asked for a fresh start.
    pub reconciling: bool,
}

/// How a page leaves the sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Round {
    /// More pages to come.
    Continues,
    /// The round is complete.
    Finished,
}

const FOLDERS: &str = "mail-folders";

fn messages_resource(folder_id: &str) -> String {
    format!("messages:{folder_id}")
}

pub struct AccountDb {
    connection: Mutex<Connection>,
}

impl AccountDb {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }

        Self::prepare(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self, StoreError> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(mut connection: Connection) -> Result<Self, StoreError> {
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
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

    // ---- sync state ----------------------------------------------------

    pub fn folders_sync(&self) -> Result<SyncState, StoreError> {
        sync_state(&self.connection(), FOLDERS)
    }

    pub fn messages_sync(&self, folder_id: &str) -> Result<SyncState, StoreError> {
        sync_state(&self.connection(), &messages_resource(folder_id))
    }

    /// The server no longer accepts the saved place in the folder tree's
    /// sync: start a full pass, keeping everything until it is done.
    pub fn restart_folders_sync(&self) -> Result<(), StoreError> {
        restart(&self.connection(), FOLDERS)
    }

    pub fn restart_messages_sync(&self, folder_id: &str) -> Result<(), StoreError> {
        restart(&self.connection(), &messages_resource(folder_id))
    }

    // ---- applying pages ------------------------------------------------

    /// Apply one page of the folder tree's sync, and where it leaves the
    /// sync, together or not at all.
    pub fn apply_folders(
        &self,
        changed: &[FolderChange],
        removed: &[String],
        link: &str,
        round: Round,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let state = sync_state(&transaction, FOLDERS)?;

        for folder in changed {
            transaction.execute(
                "INSERT INTO folders (id, parent_id, name, unread, total, hidden, generation)
                 VALUES (?1, ?2, coalesce(?3, ''), coalesce(?4, 0), coalesce(?5, 0), coalesce(?6, 0), ?7)
                 ON CONFLICT (id) DO UPDATE SET
                     parent_id = coalesce(?2, parent_id),
                     name = coalesce(?3, name),
                     unread = coalesce(?4, unread),
                     total = coalesce(?5, total),
                     hidden = coalesce(?6, hidden),
                     generation = ?7",
                params![folder.id, folder.parent_id, folder.name, folder.unread, folder.total, folder.hidden, state.generation],
            )?;
        }
        for id in removed {
            remove_folder(&transaction, id)?;
        }

        if round == Round::Finished && state.reconciling {
            // A full pass is over: a folder it did not mention is gone.
            let mut gone = transaction.prepare("SELECT id FROM folders WHERE generation < ?1")?;
            let gone: Vec<String> = gone
                .query_map([state.generation], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            for id in gone {
                remove_folder(&transaction, &id)?;
            }
        }

        advance(&transaction, FOLDERS, link, round)?;
        transaction.commit()?;

        Ok(())
    }

    /// Record which folder is the inbox, which the drafts, and so on.
    pub fn set_well_known(&self, names: &[(&str, String)]) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        transaction.execute("UPDATE folders SET well_known = NULL", [])?;
        for (name, id) in names {
            transaction.execute(
                "UPDATE folders SET well_known = ?1 WHERE id = ?2",
                params![name, id],
            )?;
        }
        transaction.commit()?;

        Ok(())
    }

    /// Apply one page of a folder's message sync, and where it leaves the
    /// sync, together or not at all.
    pub fn apply_messages(
        &self,
        folder_id: &str,
        changed: &[MessageChange],
        removed: &[String],
        link: &str,
        round: Round,
    ) -> Result<(), StoreError> {
        let resource = messages_resource(folder_id);
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let state = sync_state(&transaction, &resource)?;

        for message in changed {
            upsert_message(&transaction, folder_id, message, state.generation)?;
        }
        for id in removed {
            // Only if it is still this folder's. A message moved elsewhere is
            // "removed" here and "changed" there, in either order, and the
            // folder it moved to must not lose it.
            delete_messages(
                &transaction,
                "id = ?1 AND folder_id = ?2",
                params![id, folder_id],
            )?;
        }

        if round == Round::Finished && state.reconciling {
            delete_messages(
                &transaction,
                "folder_id = ?1 AND generation < ?2",
                params![folder_id, state.generation],
            )?;
        }

        advance(&transaction, &resource, link, round)?;
        transaction.commit()?;

        Ok(())
    }

    // ---- reading -------------------------------------------------------

    /// Every folder that is not hidden: the well-known ones first, in their
    /// usual order, then the rest by name.
    pub fn folders(&self) -> Result<Vec<Folder>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT id, parent_id, name, well_known, unread, total, hidden FROM folders
             WHERE hidden = 0
             ORDER BY CASE well_known
                 WHEN 'inbox' THEN 0 WHEN 'drafts' THEN 1 WHEN 'sentitems' THEN 2 WHEN 'deleteditems' THEN 3
                 WHEN 'archive' THEN 4 WHEN 'junkemail' THEN 5 WHEN 'outbox' THEN 6 ELSE 7 END,
                 name COLLATE NOCASE, id",
        )?;
        let folders = statement
            .query_map([], |row| {
                Ok(Folder {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    name: row.get(2)?,
                    well_known: row.get(3)?,
                    unread: row.get(4)?,
                    total: row.get(5)?,
                    hidden: row.get(6)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(folders)
    }

    /// A window of a folder's messages, newest first.
    pub fn messages(
        &self,
        folder_id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<MessageRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM messages WHERE folder_id = ?1 ORDER BY received_at DESC, id LIMIT ?3 OFFSET ?2"
        ))?;
        let rows = statement
            .query_map(params![folder_id, offset, limit], read_message)?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }

    pub fn message_count(&self, folder_id: &str) -> Result<u32, StoreError> {
        Ok(self.connection().query_row(
            "SELECT count(*) FROM messages WHERE folder_id = ?1",
            [folder_id],
            |r| r.get(0),
        )?)
    }

    pub fn message(&self, id: &str) -> Result<Option<MessageRow>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT {COLUMNS} FROM messages WHERE id = ?1"),
                [id],
                read_message,
            )
            .optional()?)
    }

    pub fn body(&self, message_id: &str) -> Result<Option<Body>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT content_type, content FROM bodies WHERE message_id = ?1",
                [message_id],
                |row| {
                    Ok(Body {
                        content_type: row.get(0)?,
                        content: row.get(1)?,
                    })
                },
            )
            .optional()?)
    }

    /// Keep a message's body, and index its text for search. `text` is the
    /// body as plain words.
    pub fn save_body(&self, message_id: &str, body: &Body, text: &str) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let changed = transaction.execute(
            "INSERT INTO bodies (message_id, content_type, content, text)
             SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM messages WHERE id = ?1)
             ON CONFLICT (message_id) DO UPDATE SET content_type = ?2, content = ?3, text = ?4",
            params![message_id, body.content_type, body.content, text],
        )?;
        // A message deleted while its body was on the way has nothing to index.
        if changed > 0 {
            index(&transaction, message_id)?;
        }
        transaction.commit()?;

        Ok(())
    }

    /// Messages matching a full-text query, best match first. `query` is in
    /// FTS5's own syntax and must already be safe to pass to it.
    pub fn search(
        &self,
        query: &str,
        folder_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MessageRow>, StoreError> {
        let connection = self.connection();
        let columns = COLUMNS
            .split(", ")
            .map(|c| format!("m.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = connection.prepare(&format!(
            "SELECT {columns} FROM messages_fts f JOIN messages m ON m.rowid = f.rowid
             WHERE messages_fts MATCH ?1 AND (?2 IS NULL OR m.folder_id = ?2)
             ORDER BY f.rank, m.received_at DESC LIMIT ?3"
        ))?;
        let rows = statement
            .query_map(params![query, folder_id, limit], read_message)?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }
}

const COLUMNS: &str = "id, folder_id, conversation_id, subject, preview, from_name, from_address, to_recipients, \
                       cc_recipients, received_at, sent_at, is_read, is_draft, has_attachments, importance, \
                       flag_status, categories, inference";

fn read_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        conversation_id: row.get(2)?,
        subject: row.get(3)?,
        preview: row.get(4)?,
        from_name: row.get(5)?,
        from_address: row.get(6)?,
        to_recipients: row.get(7)?,
        cc_recipients: row.get(8)?,
        received_at: row.get(9)?,
        sent_at: row.get(10)?,
        is_read: row.get(11)?,
        is_draft: row.get(12)?,
        has_attachments: row.get(13)?,
        importance: row.get(14)?,
        flag_status: row.get(15)?,
        categories: row.get(16)?,
        inference: row.get(17)?,
    })
}

/// A resource's sync state, as it is or as it starts: a first, full pass.
fn sync_state(connection: &Connection, resource: &str) -> Result<SyncState, StoreError> {
    let state = connection
        .query_row(
            "SELECT link, complete, generation, reconciling FROM sync_state WHERE resource = ?1",
            [resource],
            |row| {
                Ok(SyncState {
                    link: row.get(0)?,
                    complete: row.get(1)?,
                    generation: row.get(2)?,
                    reconciling: row.get(3)?,
                })
            },
        )
        .optional()?;

    Ok(state.unwrap_or(SyncState {
        link: None,
        complete: false,
        generation: 1,
        reconciling: true,
    }))
}

fn advance(
    transaction: &Transaction<'_>,
    resource: &str,
    link: &str,
    round: Round,
) -> Result<(), StoreError> {
    let finished = round == Round::Finished;
    transaction.execute(
        "INSERT INTO sync_state (resource, link, complete, reconciling, synced_at)
         VALUES (?1, ?2, ?3, NOT ?3, CASE WHEN ?3 THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') END)
         ON CONFLICT (resource) DO UPDATE SET
             link = ?2,
             complete = complete OR ?3,
             reconciling = reconciling AND NOT ?3,
             synced_at = CASE WHEN ?3 THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now') ELSE synced_at END",
        params![resource, link, finished],
    )?;

    Ok(())
}

fn restart(connection: &Connection, resource: &str) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO sync_state (resource, link, generation, reconciling) VALUES (?1, NULL, 2, 1)
         ON CONFLICT (resource) DO UPDATE SET link = NULL, generation = generation + 1, reconciling = 1",
        [resource],
    )?;

    Ok(())
}

fn remove_folder(transaction: &Transaction<'_>, id: &str) -> Result<(), StoreError> {
    delete_messages(transaction, "folder_id = ?1", params![id])?;
    transaction.execute(
        "DELETE FROM sync_state WHERE resource = ?1",
        [messages_resource(id)],
    )?;
    transaction.execute("DELETE FROM folders WHERE id = ?1", [id])?;

    Ok(())
}

/// Delete messages and their search entries together.
fn delete_messages(
    transaction: &Transaction<'_>,
    condition: &str,
    values: &[&dyn rusqlite::ToSql],
) -> Result<(), StoreError> {
    transaction.execute(
        &format!(
            "DELETE FROM messages_fts WHERE rowid IN (SELECT rowid FROM messages WHERE {condition})"
        ),
        values,
    )?;
    transaction.execute(&format!("DELETE FROM messages WHERE {condition}"), values)?;

    Ok(())
}

fn upsert_message(
    transaction: &Transaction<'_>,
    folder_id: &str,
    m: &MessageChange,
    generation: i64,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO messages (id, folder_id, conversation_id, subject, preview, from_name, from_address,
             to_recipients, cc_recipients, received_at, sent_at, is_read, is_draft, has_attachments, importance,
             flag_status, categories, inference, generation)
         VALUES (?1, ?2, ?3, coalesce(?4, ''), coalesce(?5, ''), coalesce(?6, ''), coalesce(?7, ''),
             coalesce(?8, '[]'), coalesce(?9, '[]'), coalesce(?10, ''), ?11, coalesce(?12, 0), coalesce(?13, 0),
             coalesce(?14, 0), coalesce(?15, 'normal'), coalesce(?16, 'notFlagged'), coalesce(?17, '[]'),
             coalesce(?18, 'focused'), ?19)
         ON CONFLICT (id) DO UPDATE SET
             folder_id = ?2,
             conversation_id = coalesce(?3, conversation_id),
             subject = coalesce(?4, subject),
             preview = coalesce(?5, preview),
             from_name = coalesce(?6, from_name),
             from_address = coalesce(?7, from_address),
             to_recipients = coalesce(?8, to_recipients),
             cc_recipients = coalesce(?9, cc_recipients),
             received_at = coalesce(?10, received_at),
             sent_at = coalesce(?11, sent_at),
             is_read = coalesce(?12, is_read),
             is_draft = coalesce(?13, is_draft),
             has_attachments = coalesce(?14, has_attachments),
             importance = coalesce(?15, importance),
             flag_status = coalesce(?16, flag_status),
             categories = coalesce(?17, categories),
             inference = coalesce(?18, inference),
             generation = ?19",
        params![
            m.id,
            folder_id,
            m.conversation_id,
            m.subject,
            m.preview,
            m.from_name,
            m.from_address,
            m.to_recipients,
            m.cc_recipients,
            m.received_at,
            m.sent_at,
            m.is_read,
            m.is_draft,
            m.has_attachments,
            m.importance,
            m.flag_status,
            m.categories,
            m.inference,
            generation
        ],
    )?;

    index(transaction, &m.id)
}

/// Bring a message's search entry in line with what is stored. Until its
/// body has been fetched, the preview stands in for it.
fn index(transaction: &Transaction<'_>, message_id: &str) -> Result<(), StoreError> {
    transaction.execute(
        "DELETE FROM messages_fts WHERE rowid = (SELECT rowid FROM messages WHERE id = ?1)",
        [message_id],
    )?;
    transaction.execute(
        "INSERT INTO messages_fts (rowid, subject, sender, recipients, body)
         SELECT m.rowid, m.subject, m.from_name || ' ' || m.from_address,
                m.to_recipients || ' ' || m.cc_recipients,
                coalesce((SELECT text FROM bodies WHERE message_id = m.id), m.preview)
         FROM messages m WHERE m.id = ?1",
        [message_id],
    )?;

    Ok(())
}
