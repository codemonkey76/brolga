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

const STEPS: &[&str] = &[
    "
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
",
    "
    -- Kept on this computer only: Graph has no documented favourites.
    ALTER TABLE folders ADD COLUMN favourite INTEGER NOT NULL DEFAULT 0;
",
    "
    -- A folder as conversations, and a conversation's messages.
    CREATE INDEX messages_by_conversation ON messages (conversation_id, received_at, id);
    CREATE INDEX messages_by_folder_conversation ON messages (folder_id, conversation_id, received_at DESC, id);
    -- A folder sorted by who sent it, and by subject.
    CREATE INDEX messages_by_folder_sender ON messages (folder_id, from_name COLLATE NOCASE, received_at DESC, id);
    CREATE INDEX messages_by_folder_subject ON messages (folder_id, subject COLLATE NOCASE, received_at DESC, id);
",
    "
    -- What is attached to a message, without the contents. A message is in
    -- attachment_lists once its list has been fetched, so one with nothing
    -- attached is not asked about again.
    CREATE TABLE attachment_lists (
        message_id TEXT PRIMARY KEY NOT NULL REFERENCES messages (id) ON DELETE CASCADE
    ) STRICT;

    CREATE TABLE attachments (
        message_id TEXT NOT NULL REFERENCES attachment_lists (message_id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        id TEXT NOT NULL,
        name TEXT NOT NULL DEFAULT '',
        content_type TEXT NOT NULL DEFAULT '',
        size INTEGER NOT NULL DEFAULT 0,
        is_inline INTEGER NOT NULL DEFAULT 0,
        -- file, item or reference.
        kind TEXT NOT NULL DEFAULT 'file',
        PRIMARY KEY (message_id, position)
    ) STRICT;
",
    "
    -- Senders whose remote pictures are fetched without asking. Kept on
    -- this computer only.
    CREATE TABLE trusted_senders (
        address TEXT PRIMARY KEY NOT NULL COLLATE NOCASE
    ) STRICT;
",
    "
    -- The pictures shown within a message's body, by the name the body
    -- calls each. A message is in inline_image_sets once they are fetched.
    CREATE TABLE inline_image_sets (
        message_id TEXT PRIMARY KEY NOT NULL REFERENCES messages (id) ON DELETE CASCADE
    ) STRICT;

    CREATE TABLE inline_images (
        message_id TEXT NOT NULL REFERENCES inline_image_sets (message_id) ON DELETE CASCADE,
        content_id TEXT NOT NULL,
        content_type TEXT NOT NULL,
        -- base64
        data TEXT NOT NULL,
        PRIMARY KEY (message_id, content_id)
    ) STRICT;
",
    "
    -- The outbox: each change a person makes, waiting to reach the server.
    CREATE TABLE ops (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        -- Made up here, to recognise a repeat of the same change.
        key TEXT NOT NULL UNIQUE,
        -- The action it was part of, which is undone as one.
        group_id TEXT NOT NULL,
        message_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        value TEXT NOT NULL,
        -- What it was before, to go back to.
        before TEXT NOT NULL,
        -- queued, done or failed.
        state TEXT NOT NULL DEFAULT 'queued',
        attempts INTEGER NOT NULL DEFAULT 0,
        error TEXT,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    ) STRICT;

    CREATE INDEX ops_by_state ON ops (state, id);
    CREATE INDEX ops_by_message ON ops (message_id, kind, state, id);
    CREATE INDEX ops_by_group ON ops (group_id);
",
    "
    -- Messages being written, and those waiting to be sent.
    CREATE TABLE drafts (
        -- Never used twice, so neither is an id made from it.
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        id TEXT UNIQUE,
        -- new, reply, reply_all or forward.
        kind TEXT NOT NULL,
        source_id TEXT,
        to_recipients TEXT NOT NULL DEFAULT '[]',
        cc_recipients TEXT NOT NULL DEFAULT '[]',
        bcc_recipients TEXT NOT NULL DEFAULT '[]',
        subject TEXT NOT NULL DEFAULT '',
        body TEXT NOT NULL DEFAULT '',
        -- editing, queued or failed.
        state TEXT NOT NULL DEFAULT 'editing',
        error TEXT,
        updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
        server_id TEXT,
        quoted TEXT,
        send_tried INTEGER NOT NULL DEFAULT 0
    ) STRICT;
",
    "
    -- Small things kept for the account on this computer, such as its signature.
    CREATE TABLE settings (
        key TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL
    ) STRICT;
",
    "
    -- Files attached to a draft, kept whole until the draft has been sent.
    CREATE TABLE draft_files (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        draft_id TEXT NOT NULL REFERENCES drafts (id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        content_type TEXT NOT NULL,
        size INTEGER NOT NULL,
        data BLOB NOT NULL,
        -- A request to attach it has gone out and may have reached the server.
        upload_tried INTEGER NOT NULL DEFAULT 0,
        uploaded INTEGER NOT NULL DEFAULT 0
    ) STRICT;

    CREATE INDEX draft_files_by_draft ON draft_files (draft_id, id);
",
    "
    -- Whether a draft's body is plain words (text) or formatted (html).
    ALTER TABLE drafts ADD COLUMN format TEXT NOT NULL DEFAULT 'text';
",
    "
    -- A draft is kept on the server too, so it shows in other mail
    -- programs. `rev` counts its changes here; `server_rev` is the count the
    -- server's copy was last brought up to, and -1 where it has none.
    ALTER TABLE drafts ADD COLUMN rev INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE drafts ADD COLUMN server_rev INTEGER NOT NULL DEFAULT -1;
    CREATE INDEX drafts_by_server ON drafts (server_id);
",
    "
    -- The day a flagged message is to be followed up by, as YYYY-MM-DD, or
    -- empty where it has none.
    ALTER TABLE messages ADD COLUMN flag_due TEXT NOT NULL DEFAULT '';
",
    "
    -- A picture shown within a draft's body is a file of the draft with a
    -- name the body refers to it by: <img src=\"cid:…\">.
    ALTER TABLE draft_files ADD COLUMN content_id TEXT;
",
    "
    CREATE TABLE calendars (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL DEFAULT '',
        color TEXT NOT NULL DEFAULT 'auto',
        hex_color TEXT NOT NULL DEFAULT '',
        is_default INTEGER NOT NULL DEFAULT 0,
        can_edit INTEGER NOT NULL DEFAULT 0,
        owner_name TEXT NOT NULL DEFAULT '',
        owner_address TEXT NOT NULL DEFAULT '',
        position INTEGER NOT NULL DEFAULT 0,
        -- Whether its events are shown. This computer's own business.
        shown INTEGER NOT NULL DEFAULT 1,
        -- Named in the list just read from the server.
        seen INTEGER NOT NULL DEFAULT 1
    ) STRICT;

    -- What a calendar view gives: each occurrence of a repeating event is
    -- an event of its own. Times are UTC, as 2026-10-20T03:00:00Z.
    CREATE TABLE events (
        id TEXT PRIMARY KEY NOT NULL,
        calendar_id TEXT NOT NULL,
        subject TEXT NOT NULL DEFAULT '',
        preview TEXT NOT NULL DEFAULT '',
        location TEXT NOT NULL DEFAULT '',
        start_at TEXT NOT NULL,
        end_at TEXT NOT NULL,
        is_all_day INTEGER NOT NULL DEFAULT 0,
        start_zone TEXT NOT NULL DEFAULT '',
        end_zone TEXT NOT NULL DEFAULT '',
        show_as TEXT NOT NULL DEFAULT 'busy',
        sensitivity TEXT NOT NULL DEFAULT 'normal',
        is_cancelled INTEGER NOT NULL DEFAULT 0,
        is_organizer INTEGER NOT NULL DEFAULT 0,
        organizer_name TEXT NOT NULL DEFAULT '',
        organizer_address TEXT NOT NULL DEFAULT '',
        response TEXT NOT NULL DEFAULT 'none',
        kind TEXT NOT NULL DEFAULT 'singleInstance',
        series_master_id TEXT,
        categories TEXT NOT NULL DEFAULT '[]',
        reminder_minutes INTEGER,
        join_url TEXT NOT NULL DEFAULT '',
        web_link TEXT NOT NULL DEFAULT '',
        change_key TEXT NOT NULL DEFAULT '',
        attendees TEXT NOT NULL DEFAULT '[]',
        generation INTEGER NOT NULL DEFAULT 0
    ) STRICT;

    CREATE INDEX events_by_time ON events (start_at, end_at);
    CREATE INDEX events_by_calendar ON events (calendar_id, generation);
",
    "
    -- What has been done about an event's reminder on this computer.
    CREATE TABLE reminders (
        event_id TEXT PRIMARY KEY NOT NULL,
        dismissed INTEGER NOT NULL DEFAULT 0,
        -- UTC. Until then it is not shown again.
        snoozed_until TEXT
    ) STRICT;
",
    "
    -- A message about a meeting: an invitation, a cancellation or an answer.
    ALTER TABLE messages ADD COLUMN is_meeting INTEGER NOT NULL DEFAULT 0;

    -- What such a message says of its meeting, as last read from the server.
    CREATE TABLE meeting_details (
        message_id TEXT PRIMARY KEY NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
        details TEXT NOT NULL
    ) STRICT;
",
    "
    -- Personal contacts: each whole as JSON, beside what it is listed and found by.
    CREATE TABLE contact_folders (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL
    ) STRICT;

    CREATE TABLE contacts (
        id TEXT PRIMARY KEY NOT NULL,
        -- Empty for the main Contacts folder.
        folder_id TEXT NOT NULL DEFAULT '',
        display_name TEXT NOT NULL,
        search TEXT NOT NULL,
        data TEXT NOT NULL
    ) STRICT;

    CREATE INDEX contacts_by_folder ON contacts (folder_id, display_name);
",
    "
    -- Whose name a message is to go out under, where it is not the
    -- mailbox's own: a mailbox the writer may send as or on behalf of.
    ALTER TABLE drafts ADD COLUMN from_address TEXT NOT NULL DEFAULT '';
",
    "
    -- The files attached to messages that were brought in from an archive
    -- file. Mail from the server keeps none here: its attachments are
    -- fetched when asked for. An archive has nowhere else to fetch from.
    CREATE TABLE attachment_content (
        message_id TEXT NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
        attachment_id TEXT NOT NULL,
        content BLOB NOT NULL,
        PRIMARY KEY (message_id, attachment_id)
    ) STRICT;
",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub well_known: Option<String>,
    pub unread: i64,
    pub total: i64,
    pub hidden: bool,
    /// Pinned to the top of the folder list, on this computer only.
    pub favourite: bool,
    /// How far below the top of the tree: 0 for the inbox and its peers.
    pub depth: u32,
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
    /// The day it is to be followed up by, `YYYY-MM-DD`, or empty.
    pub flag_due: String,
    /// It is about a meeting: an invitation, a cancellation or an answer.
    pub is_meeting: bool,
    /// JSON: an array of strings.
    pub categories: String,
    pub inference: String,
}

/// Something attached to a message, without its contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentRow {
    pub id: String,
    pub name: String,
    pub content_type: String,
    /// In bytes.
    pub size: i64,
    pub is_inline: bool,
    /// `file`, `item` or `reference`.
    pub kind: String,
}

/// A picture shown within a message's body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    /// The name the body calls it by, without the `cid:`.
    pub content_id: String,
    pub content_type: String,
    /// base64
    pub data: String,
}

/// The order a folder's messages are listed in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Order {
    #[default]
    Newest,
    Oldest,
    /// By the sender's name, and newest first for each sender.
    Sender,
    /// By subject, and newest first for each subject.
    Subject,
}

impl Order {
    fn sql(self) -> &'static str {
        match self {
            Self::Newest => "received_at DESC, id",
            Self::Oldest => "received_at ASC, id DESC",
            Self::Sender => "from_name COLLATE NOCASE, received_at DESC, id",
            Self::Subject => "subject COLLATE NOCASE, received_at DESC, id",
        }
    }
}

/// A conversation as one line of a folder: its newest message there, and
/// how many of the folder's messages belong to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRow {
    pub latest: MessageRow,
    pub count: u32,
    pub unread: u32,
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
    /// `YYYY-MM-DD`, or empty for none.
    pub flag_due: Option<String>,
    pub is_meeting: Option<bool>,
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

    pub(crate) fn connection(&self) -> MutexGuard<'_, Connection> {
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

    /// How much of the mailbox is here, and how recently it was brought up
    /// to date: (folders, messages, folders whose first sync has finished,
    /// the oldest "last synced" among those).
    pub fn sync_summary(&self) -> Result<(u32, u32, u32, Option<String>), StoreError> {
        let connection = self.connection();
        let folders =
            connection.query_row("SELECT count(*) FROM folders WHERE hidden = 0", [], |r| {
                r.get(0)
            })?;
        let messages = connection.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))?;
        let (complete, oldest) = connection.query_row(
            "SELECT count(*), min(synced_at) FROM sync_state WHERE resource LIKE 'messages:%' AND complete = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;

        Ok((folders, messages, complete, oldest))
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
        let ids: Vec<&str> = changed.iter().map(|m| m.id.as_str()).collect();
        crate::ops::keep_queued_edits(&transaction, &ids)?;
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
            // A full pass is over: a message it did not mention is gone.
            // One moved here on this computer, whose move the server has
            // not had yet, was never going to be mentioned, and stays.
            delete_messages(
                &transaction,
                "folder_id = ?1 AND generation < ?2 AND id NOT IN (
                     SELECT message_id FROM ops WHERE state = 'queued' AND kind = 'folder_id')",
                params![folder_id, state.generation],
            )?;
        }

        advance(&transaction, &resource, link, round)?;
        transaction.commit()?;

        Ok(())
    }

    // ---- reading -------------------------------------------------------

    /// Every folder that is not hidden, as a tree read from the top down:
    /// each folder is followed by the folders inside it. At every level the
    /// well-known ones come first, in their usual order, then the rest by
    /// name.
    pub fn folders(&self) -> Result<Vec<Folder>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT id, parent_id, name, well_known, unread, total, hidden, favourite FROM folders
             WHERE hidden = 0
             ORDER BY CASE well_known
                 WHEN 'inbox' THEN 0 WHEN 'drafts' THEN 1 WHEN 'sentitems' THEN 2 WHEN 'deleteditems' THEN 3
                 WHEN 'archive' THEN 4 WHEN 'junkemail' THEN 5 WHEN 'outbox' THEN 6 ELSE 7 END,
                 name COLLATE NOCASE, id",
        )?;
        let folders: Vec<Folder> = statement
            .query_map([], |row| {
                Ok(Folder {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    name: row.get(2)?,
                    well_known: row.get(3)?,
                    unread: row.get(4)?,
                    total: row.get(5)?,
                    hidden: row.get(6)?,
                    favourite: row.get(7)?,
                    depth: 0,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(as_tree(folders))
    }

    /// People to offer while an address is being typed: those whose name or
    /// address has a word starting with what was typed, the ones written to
    /// most first, then the ones heard from most. Gives `(name, address)`.
    pub fn suggest_people(
        &self,
        typed: &str,
        limit: u32,
    ) -> Result<Vec<(String, String)>, StoreError> {
        let typed = typed.trim().to_lowercase();
        if typed.is_empty() {
            return Ok(Vec::new());
        }
        // `%` and `_` typed by a person are letters, not wildcards.
        let escaped = typed
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let starts = format!("{escaped}%");
        let within = format!("% {escaped}%");
        let after_dot = format!("%.{escaped}%");

        let connection = self.connection();
        let mut statement = connection.prepare(
            "WITH seen (name, address, weight) AS (
                 -- Written to: every recipient of what is in Sent Items.
                 SELECT json_extract(r.value, '$.name'), json_extract(r.value, '$.address'), 5
                 FROM messages m, json_each(m.to_recipients || '') r
                 WHERE m.folder_id = (SELECT id FROM folders WHERE well_known = 'sentitems')
                 UNION ALL
                 SELECT json_extract(r.value, '$.name'), json_extract(r.value, '$.address'), 5
                 FROM messages m, json_each(m.cc_recipients || '') r
                 WHERE m.folder_id = (SELECT id FROM folders WHERE well_known = 'sentitems')
                 UNION ALL
                 -- Heard from.
                 SELECT from_name, from_address, 1 FROM messages
             )
             SELECT max(coalesce(name, '')), lower(address), sum(weight) AS score
             FROM seen
             WHERE address IS NOT NULL AND address LIKE '%@%' AND (
                 lower(address) LIKE ?1 ESCAPE '\\'
                 OR lower(coalesce(name, '')) LIKE ?1 ESCAPE '\\'
                 OR lower(coalesce(name, '')) LIKE ?2 ESCAPE '\\'
                 OR lower(address) LIKE ?3 ESCAPE '\\')
             GROUP BY lower(address)
             ORDER BY score DESC, lower(address)
             LIMIT ?4",
        )?;
        let people = statement
            .query_map(params![starts, within, after_dot, limit], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<_, _>>()?;

        Ok(people)
    }

    /// Something kept for the account on this computer, by its name.
    pub fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;

        Ok(())
    }

    /// Take a message off this computer, with everything kept about it,
    /// because it is gone from the server for good. Its folder's counts
    /// follow. False if it was not here.
    pub fn forget_message(&self, id: &str) -> Result<bool, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE folders SET
                 total = max(0, total - 1),
                 unread = max(0, unread - (SELECT NOT is_read FROM messages WHERE id = ?1))
             WHERE id = (SELECT folder_id FROM messages WHERE id = ?1)",
            [id],
        )?;
        let here: bool = transaction.query_row(
            "SELECT EXISTS (SELECT 1 FROM messages WHERE id = ?1)",
            [id],
            |row| row.get(0),
        )?;
        // Anything still queued for it has nothing left to change.
        transaction.execute(
            "DELETE FROM ops WHERE message_id = ?1 AND state = 'queued'",
            [id],
        )?;
        delete_messages(&transaction, "id = ?1", params![id])?;
        transaction.commit()?;

        Ok(here)
    }

    /// A folder's id, given its id or, for a standard folder, its
    /// well-known name.
    pub fn folder_id(&self, folder: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT id FROM folders WHERE id = ?1 OR well_known = ?1",
                [folder],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Pin a folder to the top of the list, or unpin it. The folder may be
    /// named by its id or, for a standard folder, by its well-known name.
    /// Gives back whether there is such a folder.
    pub fn set_favourite(&self, folder: &str, favourite: bool) -> Result<bool, StoreError> {
        let changed = self.connection().execute(
            "UPDATE folders SET favourite = ?2 WHERE id = ?1 OR well_known = ?1",
            params![folder, favourite],
        )?;

        Ok(changed > 0)
    }

    /// A window of a folder's messages, newest first.
    pub fn messages(
        &self,
        folder_id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<MessageRow>, StoreError> {
        self.messages_in_order(folder_id, Order::Newest, offset, limit)
    }

    /// A window of a folder's messages, in the order asked for.
    pub fn messages_in_order(
        &self,
        folder_id: &str,
        order: Order,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<MessageRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM messages WHERE folder_id = ?1 ORDER BY {} LIMIT ?3 OFFSET ?2",
            order.sql()
        ))?;
        let rows = statement
            .query_map(params![folder_id, offset, limit], read_message)?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }

    /// A window of a folder's conversations, the one with the newest message
    /// first. A conversation is counted within the folder: its messages in
    /// other folders (the replies in Sent Items, say) are not this folder's.
    pub fn conversations(
        &self,
        folder_id: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<ConversationRow>, StoreError> {
        let connection = self.connection();
        // A message stands for its conversation when it is the newest of it
        // in the folder. One without a conversation stands for itself.
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS},
                 CASE WHEN m.conversation_id IS NULL THEN 1 ELSE (
                     SELECT count(*) FROM messages c
                     WHERE c.folder_id = m.folder_id AND c.conversation_id = m.conversation_id) END,
                 CASE WHEN m.conversation_id IS NULL THEN NOT m.is_read ELSE (
                     SELECT count(*) FROM messages c
                     WHERE c.folder_id = m.folder_id AND c.conversation_id = m.conversation_id AND c.is_read = 0) END
             FROM messages m
             WHERE m.folder_id = ?1 AND (m.conversation_id IS NULL OR m.id = (
                 SELECT l.id FROM messages l
                 WHERE l.folder_id = m.folder_id AND l.conversation_id = m.conversation_id
                 ORDER BY l.received_at DESC, l.id LIMIT 1))
             ORDER BY m.received_at DESC, m.id LIMIT ?3 OFFSET ?2"
        ))?;
        let rows = statement
            .query_map(params![folder_id, offset, limit], |row| {
                Ok(ConversationRow {
                    latest: read_message(row)?,
                    count: row.get(20)?,
                    unread: row.get(21)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }

    pub fn conversation_count(&self, folder_id: &str) -> Result<u32, StoreError> {
        Ok(self.connection().query_row(
            "SELECT count(DISTINCT conversation_id) + count(*) FILTER (WHERE conversation_id IS NULL)
             FROM messages WHERE folder_id = ?1",
            [folder_id],
            |r| r.get(0),
        )?)
    }

    /// Every message of a conversation, in whichever folders they are,
    /// oldest first: the order it was written in.
    pub fn conversation(&self, conversation_id: &str) -> Result<Vec<MessageRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM messages WHERE conversation_id = ?1 ORDER BY received_at, id"
        ))?;
        let rows = statement
            .query_map([conversation_id], read_message)?
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

    /// The pictures within a message's body, or nothing if they have not
    /// been fetched yet.
    pub fn inline_images(&self, message_id: &str) -> Result<Option<Vec<InlineImage>>, StoreError> {
        let connection = self.connection();
        let fetched: bool = connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM inline_image_sets WHERE message_id = ?1)",
            [message_id],
            |row| row.get(0),
        )?;
        if !fetched {
            return Ok(None);
        }

        let mut statement = connection.prepare(
            "SELECT content_id, content_type, data FROM inline_images WHERE message_id = ?1",
        )?;
        let rows = statement
            .query_map([message_id], |row| {
                Ok(InlineImage {
                    content_id: row.get(0)?,
                    content_type: row.get(1)?,
                    data: row.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(Some(rows))
    }

    pub fn save_inline_images(
        &self,
        message_id: &str,
        images: &[InlineImage],
    ) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM inline_image_sets WHERE message_id = ?1",
            [message_id],
        )?;
        let known = transaction.execute(
            "INSERT INTO inline_image_sets (message_id) SELECT ?1 WHERE EXISTS (SELECT 1 FROM messages WHERE id = ?1)",
            [message_id],
        )?;
        if known > 0 {
            for each in images {
                transaction.execute(
                    "INSERT OR IGNORE INTO inline_images (message_id, content_id, content_type, data) VALUES (?1, ?2, ?3, ?4)",
                    params![message_id, each.content_id, each.content_type, each.data],
                )?;
            }
        }
        transaction.commit()?;

        Ok(())
    }

    /// What a message about a meeting says of it, as JSON, if that has
    /// been read from the server.
    pub fn meeting_details(&self, message_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT details FROM meeting_details WHERE message_id = ?1",
                [message_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn save_meeting_details(&self, message_id: &str, details: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO meeting_details (message_id, details)
             SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM messages WHERE id = ?1)
             ON CONFLICT (message_id) DO UPDATE SET details = ?2",
            params![message_id, details],
        )?;

        Ok(())
    }

    /// Whose remote pictures are fetched without asking, in order.
    pub fn trusted_senders(&self) -> Result<Vec<String>, StoreError> {
        let connection = self.connection();
        let mut statement =
            connection.prepare("SELECT address FROM trusted_senders ORDER BY address")?;
        let rows = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }

    pub fn is_trusted_sender(&self, address: &str) -> Result<bool, StoreError> {
        Ok(self.connection().query_row(
            "SELECT EXISTS (SELECT 1 FROM trusted_senders WHERE address = ?1)",
            [address.trim()],
            |row| row.get(0),
        )?)
    }

    pub fn set_trusted_sender(&self, address: &str, trusted: bool) -> Result<(), StoreError> {
        let address = address.trim();
        let connection = self.connection();
        if trusted {
            connection.execute(
                "INSERT OR IGNORE INTO trusted_senders (address) VALUES (?1)",
                [address],
            )?;
        } else {
            connection.execute("DELETE FROM trusted_senders WHERE address = ?1", [address])?;
        }

        Ok(())
    }

    /// What is attached to a message, or nothing if that has not been
    /// fetched yet.
    pub fn attachments(&self, message_id: &str) -> Result<Option<Vec<AttachmentRow>>, StoreError> {
        let connection = self.connection();
        let listed: bool = connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM attachment_lists WHERE message_id = ?1)",
            [message_id],
            |row| row.get(0),
        )?;
        if !listed {
            return Ok(None);
        }

        let mut statement = connection.prepare(
            "SELECT id, name, content_type, size, is_inline, kind FROM attachments
             WHERE message_id = ?1 ORDER BY position",
        )?;
        let rows = statement
            .query_map([message_id], |row| {
                Ok(AttachmentRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    content_type: row.get(2)?,
                    size: row.get(3)?,
                    is_inline: row.get(4)?,
                    kind: row.get(5)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(Some(rows))
    }

    /// Keep what is attached to a message. Nothing is kept for a message
    /// deleted while the list was on its way.
    pub fn save_attachments(
        &self,
        message_id: &str,
        attachments: &[AttachmentRow],
    ) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM attachment_lists WHERE message_id = ?1",
            [message_id],
        )?;
        let known = transaction.execute(
            "INSERT INTO attachment_lists (message_id) SELECT ?1 WHERE EXISTS (SELECT 1 FROM messages WHERE id = ?1)",
            [message_id],
        )?;
        if known > 0 {
            for (position, each) in (0i64..).zip(attachments) {
                transaction.execute(
                    "INSERT INTO attachments (message_id, position, id, name, content_type, size, is_inline, kind)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![message_id, position, each.id, each.name, each.content_type, each.size, each.is_inline, each.kind],
                )?;
            }
        }
        transaction.commit()?;

        Ok(())
    }

    /// Messages received since a date whose bodies are not here yet, newest first.
    pub fn missing_bodies(&self, since: &str, limit: u32) -> Result<Vec<String>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT m.id FROM messages m
             WHERE m.received_at >= ?1 AND NOT EXISTS (SELECT 1 FROM bodies b WHERE b.message_id = m.id)
             ORDER BY m.received_at DESC, m.id LIMIT ?2",
        )?;
        let ids = statement
            .query_map(params![since, limit], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        Ok(ids)
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

    /// The latest messages from one address, newest first, wherever they are kept.
    pub fn messages_from(&self, address: &str, limit: u32) -> Result<Vec<MessageRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM messages WHERE from_address = ?1 COLLATE NOCASE AND is_draft = 0
             ORDER BY received_at DESC, id LIMIT ?2"
        ))?;
        let rows = statement
            .query_map(params![address, limit], read_message)?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }

    /// Messages matching a search, best match first, or newest first where
    /// the search is filters only. `fts` is in FTS5's own syntax and must
    /// already be safe to pass to it.
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        fts: &str,
        folder_id: Option<&str>,
        has_attachment: Option<bool>,
        is_read: Option<bool>,
        after: Option<&str>,
        before: Option<&str>,
        limit: u32,
    ) -> Result<Vec<MessageRow>, StoreError> {
        let connection = self.connection();
        let columns = COLUMNS
            .split(", ")
            .map(|c| format!("m.{}", c.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let filters = "(?2 IS NULL OR m.folder_id = ?2)
             AND (?3 IS NULL OR m.has_attachments = ?3)
             AND (?4 IS NULL OR m.is_read = ?4)
             AND (?5 IS NULL OR m.received_at >= ?5)
             AND (?6 IS NULL OR m.received_at < ?6)";
        let sql = if fts.is_empty() {
            format!(
                "SELECT {columns} FROM messages m WHERE ?1 = '' AND {filters} ORDER BY m.received_at DESC, m.id LIMIT ?7"
            )
        } else {
            format!(
                "SELECT {columns} FROM messages_fts f JOIN messages m ON m.rowid = f.rowid
                 WHERE messages_fts MATCH ?1 AND {filters}
                 ORDER BY f.rank, m.received_at DESC LIMIT ?7"
            )
        };
        let mut statement = connection.prepare(&sql)?;
        let rows = statement
            .query_map(
                params![
                    fts,
                    folder_id,
                    has_attachment,
                    is_read,
                    after,
                    before,
                    limit
                ],
                read_message,
            )?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }
}

const COLUMNS: &str = "id, folder_id, conversation_id, subject, preview, from_name, from_address, to_recipients, \
                       cc_recipients, received_at, sent_at, is_read, is_draft, has_attachments, importance, \
                       flag_status, categories, inference, flag_due, is_meeting";

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
        flag_due: row.get(18)?,
        is_meeting: row.get(19)?,
    })
}

/// A resource's sync state, as it is or as it starts: a first, full pass.
pub(crate) fn sync_state(connection: &Connection, resource: &str) -> Result<SyncState, StoreError> {
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

pub(crate) fn advance(
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

pub(crate) fn restart(connection: &Connection, resource: &str) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO sync_state (resource, link, generation, reconciling) VALUES (?1, NULL, 2, 1)
         ON CONFLICT (resource) DO UPDATE SET link = NULL, generation = generation + 1, reconciling = 1",
        [resource],
    )?;

    Ok(())
}

/// Put folders, already in the order their level should read, into the
/// order of the whole tree. A folder whose parent is not among them (the
/// mailbox's own root is not) sits at the top. The server's word is not
/// trusted to be a tree: a folder is placed once, so a loop of parents
/// cannot hang this, and whatever a loop leaves unplaced goes at the top.
fn as_tree(folders: Vec<Folder>) -> Vec<Folder> {
    use std::collections::{HashMap, HashSet};

    let ids: HashSet<&str> = folders.iter().map(|f| f.id.as_str()).collect();
    let mut children: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut tops = Vec::new();
    for (index, folder) in folders.iter().enumerate() {
        match folder.parent_id.as_deref() {
            Some(parent) if parent != folder.id && ids.contains(parent) => {
                children.entry(parent).or_default().push(index);
            }
            _ => tops.push(index),
        }
    }

    let mut placed = vec![false; folders.len()];
    let mut order = Vec::with_capacity(folders.len());
    let mut place = |start: usize, placed: &mut Vec<bool>| {
        let mut waiting = vec![(start, 0u32)];
        while let Some((index, depth)) = waiting.pop() {
            if std::mem::replace(&mut placed[index], true) {
                continue;
            }
            order.push((index, depth));
            if let Some(inside) = children.get(folders[index].id.as_str()) {
                waiting.extend(inside.iter().rev().map(|&child| (child, depth + 1)));
            }
        }
    };
    for top in tops {
        place(top, &mut placed);
    }
    for index in 0..folders.len() {
        if !placed[index] {
            place(index, &mut placed);
        }
    }

    let mut folders: Vec<Option<Folder>> = folders.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|(index, depth)| {
            let mut folder = folders.get_mut(index)?.take()?;
            folder.depth = depth;
            Some(folder)
        })
        .collect()
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

pub(crate) fn upsert_message(
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
    if let Some(is_meeting) = m.is_meeting {
        transaction.execute(
            "UPDATE messages SET is_meeting = ?2 WHERE id = ?1",
            params![m.id, is_meeting],
        )?;
    }
    if let Some(due) = &m.flag_due {
        transaction.execute(
            "UPDATE messages SET flag_due = ?2 WHERE id = ?1",
            params![m.id, due],
        )?;
    }

    index(transaction, &m.id)
}

/// Bring a message's search entry in line with what is stored. Until its
/// body has been fetched, the preview stands in for it.
pub(crate) fn index(transaction: &Transaction<'_>, message_id: &str) -> Result<(), StoreError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: &str, parent: &str) -> FolderChange {
        FolderChange {
            id: id.to_owned(),
            parent_id: Some(parent.to_owned()),
            name: Some(id.to_owned()),
            ..FolderChange::default()
        }
    }

    fn with_folders(folders: &[FolderChange]) -> AccountDb {
        let db = AccountDb::in_memory().unwrap();
        db.apply_folders(folders, &[], "next", Round::Finished)
            .unwrap();
        db
    }

    fn shape(db: &AccountDb) -> Vec<(String, u32)> {
        db.folders()
            .unwrap()
            .into_iter()
            .map(|f| (f.id, f.depth))
            .collect()
    }

    #[test]
    fn folders_read_as_a_tree_with_the_standard_ones_first() {
        let db = with_folders(&[
            folder("zebra", "root"),
            folder("inbox-id", "root"),
            folder("clients", "inbox-id"),
            folder("acme", "clients"),
            folder("billing", "inbox-id"),
            folder("apple", "root"),
        ]);
        db.set_well_known(&[("inbox", "inbox-id".to_owned())])
            .unwrap();

        assert_eq!(
            shape(&db),
            [
                ("inbox-id".to_owned(), 0),
                ("billing".to_owned(), 1),
                ("clients".to_owned(), 1),
                ("acme".to_owned(), 2),
                ("apple".to_owned(), 0),
                ("zebra".to_owned(), 0),
            ]
        );
    }

    #[test]
    fn folders_that_claim_each_other_as_parent_are_still_all_listed() {
        let db = with_folders(&[
            folder("a", "b"),
            folder("b", "a"),
            folder("own", "own"),
            folder("plain", "root"),
        ]);

        let mut ids: Vec<String> = shape(&db).into_iter().map(|(id, _)| id).collect();
        ids.sort();

        assert_eq!(ids, ["a", "b", "own", "plain"]);
    }

    #[test]
    fn a_favourite_stays_one_through_a_sync_and_goes_with_its_folder() {
        let db = with_folders(&[folder("projects", "root")]);

        assert!(db.set_favourite("projects", true).unwrap());
        assert!(!db.set_favourite("no-such-folder", true).unwrap());

        db.apply_folders(&[folder("projects", "root")], &[], "next", Round::Finished)
            .unwrap();
        assert!(db.folders().unwrap()[0].favourite);

        db.apply_folders(&[], &["projects".to_owned()], "next", Round::Finished)
            .unwrap();
        assert!(db.folders().unwrap().is_empty());
    }

    fn mail(
        id: &str,
        conversation: Option<&str>,
        from: &str,
        subject: &str,
        received: &str,
        read: bool,
    ) -> MessageChange {
        MessageChange {
            id: id.to_owned(),
            conversation_id: conversation.map(str::to_owned),
            from_name: Some(from.to_owned()),
            subject: Some(subject.to_owned()),
            received_at: Some(received.to_owned()),
            is_read: Some(read),
            ..MessageChange::default()
        }
    }

    fn with_mail() -> AccountDb {
        let db = AccountDb::in_memory().unwrap();
        db.apply_messages(
            "inbox",
            &[
                mail(
                    "1",
                    Some("lunch"),
                    "Sam",
                    "Lunch?",
                    "2026-10-01T09:00:00Z",
                    true,
                ),
                mail(
                    "2",
                    Some("budget"),
                    "alex",
                    "Budget",
                    "2026-10-02T09:00:00Z",
                    true,
                ),
                mail(
                    "3",
                    Some("lunch"),
                    "Robin",
                    "RE: Lunch?",
                    "2026-10-03T09:00:00Z",
                    false,
                ),
                mail("4", None, "Zoe", "apples", "2026-10-04T09:00:00Z", false),
                mail(
                    "5",
                    Some("lunch"),
                    "Sam",
                    "RE: Lunch?",
                    "2026-10-05T09:00:00Z",
                    false,
                ),
            ],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();
        db.apply_messages(
            "sent",
            &[mail(
                "6",
                Some("lunch"),
                "Me",
                "RE: Lunch?",
                "2026-10-04T12:00:00Z",
                true,
            )],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();
        db
    }

    fn ids(rows: &[MessageRow]) -> Vec<&str> {
        rows.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn people_are_suggested_by_how_a_name_or_address_starts_and_those_written_to_first() {
        let db = AccountDb::in_memory().unwrap();
        db.apply_folders(
            &[folder("sent", "root"), folder("inbox", "root")],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();
        db.set_well_known(&[("sentitems", "sent".to_owned())])
            .unwrap();
        let heard = |id: &str, name: &str, address: &str| MessageChange {
            id: id.to_owned(),
            from_name: Some(name.to_owned()),
            from_address: Some(address.to_owned()),
            ..MessageChange::default()
        };
        db.apply_messages(
            "inbox",
            &[
                heard("1", "Sam Taylor", "sam.taylor@example.test"),
                heard("2", "Sam Taylor", "Sam.Taylor@example.test"),
                heard("3", "Samira Khan", "s.khan@example.test"),
                heard("4", "Newsletter", "news@sample.test"),
                heard("5", "100% Offers", "offers@example.test"),
            ],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();
        db.apply_messages(
            "sent",
            &[MessageChange {
                id: "6".to_owned(),
                to_recipients: Some(
                    r#"[{"name":"Samira Khan","address":"s.khan@example.test"}]"#.to_owned(),
                ),
                cc_recipients: Some(r#"[{"name":null,"address":"jo@example.test"}]"#.to_owned()),
                ..MessageChange::default()
            }],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();
        let suggest = |typed: &str| -> Vec<String> {
            db.suggest_people(typed, 5)
                .unwrap()
                .into_iter()
                .map(|(_, address)| address)
                .collect()
        };

        // Samira was written to, so she is first though Sam wrote more often.
        assert_eq!(
            suggest("sam"),
            ["s.khan@example.test", "sam.taylor@example.test"]
        );
        assert_eq!(suggest("TAY"), ["sam.taylor@example.test"]);
        assert_eq!(suggest("khan"), ["s.khan@example.test"]);
        assert_eq!(suggest("jo"), ["jo@example.test"]);
        assert_eq!(db.suggest_people("sam.t", 5).unwrap()[0].0, "Sam Taylor");
        // What is typed is letters, not a pattern.
        assert_eq!(suggest("%"), Vec::<String>::new());
        assert_eq!(suggest("100%"), ["offers@example.test"]);
        assert!(suggest("  ").is_empty());
    }

    #[test]
    fn a_folder_can_be_listed_in_each_order() {
        let db = with_mail();
        let listed = |order| db.messages_in_order("inbox", order, 0, 10).unwrap();

        assert_eq!(ids(&listed(Order::Newest)), ["5", "4", "3", "2", "1"]);
        assert_eq!(ids(&listed(Order::Oldest)), ["1", "2", "3", "4", "5"]);
        assert_eq!(ids(&listed(Order::Sender)), ["2", "3", "5", "1", "4"]);
        assert_eq!(ids(&listed(Order::Subject)), ["4", "2", "1", "5", "3"]);
    }

    #[test]
    fn a_folder_as_conversations_has_one_line_for_each() {
        let db = with_mail();

        let lines = db.conversations("inbox", 0, 10).unwrap();
        let shape: Vec<(&str, u32, u32)> = lines
            .iter()
            .map(|c| (c.latest.id.as_str(), c.count, c.unread))
            .collect();

        // The lunch conversation has a fourth message, in Sent, which is
        // not the inbox's to count.
        assert_eq!(shape, [("5", 3, 2), ("4", 1, 1), ("2", 1, 0)]);
        assert_eq!(db.conversation_count("inbox").unwrap(), 3);
        assert_eq!(
            ids(&db
                .conversations("inbox", 1, 1)
                .unwrap()
                .into_iter()
                .map(|c| c.latest)
                .collect::<Vec<_>>()),
            ["4"]
        );
    }

    #[test]
    fn a_conversation_is_read_across_folders_in_the_order_it_was_written() {
        let db = with_mail();

        assert_eq!(
            ids(&db.conversation("lunch").unwrap()),
            ["1", "3", "6", "5"]
        );
    }
}
