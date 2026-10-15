//! Messages being written, and those waiting to be sent.
//!
//! A draft lives here from its first keystroke, so it is not lost to a
//! crash or to being offline. Sending one queues it; it stays queued, and
//! in this table, until the server has confirmed it sent.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{AccountDb, StoreError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub id: String,
    /// `new`, `reply`, `reply_all` or `forward`.
    pub kind: String,
    /// The message replied to or forwarded.
    pub source_id: Option<String>,
    /// JSON: arrays of `{name, address}`.
    pub to_recipients: String,
    pub cc_recipients: String,
    pub bcc_recipients: String,
    pub subject: String,
    /// The words as typed: plain, or as HTML, as `format` says.
    pub body: String,
    /// `text` or `html`.
    pub format: String,
    /// The address it is to go out under, where that is not the
    /// mailbox's own. Empty for the mailbox's own.
    pub from_address: String,
    /// `editing`, `queued` (waiting to be sent) or `failed` (the server
    /// would not send it; it can be put right and sent again).
    pub state: String,
    pub error: Option<String>,
    /// Seconds since 1970.
    pub updated_at: i64,
    /// The draft's id on the server, once it has one.
    pub server_id: Option<String>,
    /// The original as the server quotes it under a reply or a forward.
    pub quoted: Option<String>,
    /// A request to send has gone out and may have reached the server.
    pub send_tried: bool,
    /// How many times it has been changed here.
    pub rev: i64,
    /// The count the server's copy was last brought up to; -1 where the
    /// server has none.
    pub server_rev: i64,
}

/// A file attached to a draft, without its contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftFile {
    pub id: i64,
    pub name: String,
    pub content_type: String,
    /// In bytes.
    pub size: i64,
    /// A request to attach it has gone out and may have reached the server.
    pub upload_tried: bool,
    /// The server has it.
    pub uploaded: bool,
    /// For a picture shown within the body: the name the body refers to
    /// it by, without the `cid:`.
    pub content_id: Option<String>,
    /// Its id on the server's copy of the draft, where that is known.
    pub attachment_id: Option<String>,
}

/// What may be changed about a draft. `None` leaves a part as it is.
#[derive(Debug, Clone, Default)]
pub struct DraftEdit {
    pub to_recipients: Option<String>,
    pub cc_recipients: Option<String>,
    pub bcc_recipients: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
    /// `text` or `html`.
    pub format: Option<String>,
}

const COLUMNS: &str = "id, kind, source_id, to_recipients, cc_recipients, bcc_recipients, subject, body, state, \
                       error, updated_at, server_id, quoted, send_tried, format, rev, server_rev, from_address";

fn read_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<DraftFile> {
    Ok(DraftFile {
        id: row.get(0)?,
        name: row.get(1)?,
        content_type: row.get(2)?,
        size: row.get(3)?,
        upload_tried: row.get(4)?,
        uploaded: row.get(5)?,
        content_id: row.get(6)?,
        attachment_id: row.get(7)?,
    })
}

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Draft> {
    Ok(Draft {
        id: row.get(0)?,
        kind: row.get(1)?,
        source_id: row.get(2)?,
        to_recipients: row.get(3)?,
        cc_recipients: row.get(4)?,
        bcc_recipients: row.get(5)?,
        subject: row.get(6)?,
        body: row.get(7)?,
        state: row.get(8)?,
        error: row.get(9)?,
        updated_at: row.get(10)?,
        server_id: row.get(11)?,
        quoted: row.get(12)?,
        send_tried: row.get(13)?,
        format: row.get(14)?,
        rev: row.get(15)?,
        server_rev: row.get(16)?,
        from_address: row.get(17)?,
    })
}

impl AccountDb {
    /// Start a draft, and give it back with its id.
    pub fn create_draft(
        &self,
        kind: &str,
        source_id: Option<&str>,
        start: &DraftEdit,
    ) -> Result<Draft, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO drafts (kind, source_id, to_recipients, cc_recipients, bcc_recipients, subject, body, format)
             VALUES (?1, ?2, coalesce(?3, '[]'), coalesce(?4, '[]'), coalesce(?5, '[]'), coalesce(?6, ''), coalesce(?7, ''),
                     coalesce(?8, 'text'))",
            params![
                kind,
                source_id,
                start.to_recipients,
                start.cc_recipients,
                start.bcc_recipients,
                start.subject,
                start.body,
                start.format
            ],
        )?;
        let id = format!("draft-{}", transaction.last_insert_rowid());
        transaction.execute(
            "UPDATE drafts SET id = ?1 WHERE seq = last_insert_rowid()",
            [&id],
        )?;
        transaction.commit()?;

        Ok(connection.query_row(
            &format!("SELECT {COLUMNS} FROM drafts WHERE id = ?1"),
            [&id],
            read,
        )?)
    }

    pub fn draft(&self, id: &str) -> Result<Option<Draft>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT {COLUMNS} FROM drafts WHERE id = ?1"),
                [id],
                read,
            )
            .optional()?)
    }

    /// Every draft, the most recently touched first.
    pub fn drafts(&self) -> Result<Vec<Draft>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM drafts ORDER BY updated_at DESC, seq DESC"
        ))?;
        let drafts = statement.query_map([], read)?.collect::<Result<_, _>>()?;

        Ok(drafts)
    }

    /// Change a draft that is not on its way out. A draft that failed to
    /// send goes back to being edited. False if there is no such draft or
    /// it is queued.
    /// Say whose name a draft is to go out under: an address, or empty
    /// for the mailbox's own. False if there is no such draft, or it is queued.
    pub fn set_draft_from(&self, id: &str, address: &str) -> Result<bool, StoreError> {
        let changed = self.connection().execute(
            "UPDATE drafts SET from_address = ?2, rev = rev + 1, state = 'editing', error = NULL, updated_at = unixepoch()
             WHERE id = ?1 AND state <> 'queued'",
            params![id, address],
        )?;

        Ok(changed > 0)
    }

    pub fn edit_draft(&self, id: &str, edit: &DraftEdit) -> Result<bool, StoreError> {
        let changed = self.connection().execute(
            "UPDATE drafts SET
                 to_recipients = coalesce(?2, to_recipients),
                 cc_recipients = coalesce(?3, cc_recipients),
                 bcc_recipients = coalesce(?4, bcc_recipients),
                 subject = coalesce(?5, subject),
                 body = coalesce(?6, body),
                 format = coalesce(?7, format),
                 rev = rev + 1,
                 state = 'editing', error = NULL, updated_at = unixepoch()
             WHERE id = ?1 AND state <> 'queued'",
            params![
                id,
                edit.to_recipients,
                edit.cc_recipients,
                edit.bcc_recipients,
                edit.subject,
                edit.body,
                edit.format
            ],
        )?;

        Ok(changed > 0)
    }

    /// Attach a file to a draft that is not on its way out. Nothing if
    /// there is no such draft or it is queued.
    pub fn attach_to_draft(
        &self,
        draft_id: &str,
        name: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<Option<DraftFile>, StoreError> {
        self.attach(draft_id, name, content_type, data, false)
    }

    /// Attach a picture to be shown within the body. It is given a name
    /// for the body to refer to it by, which comes back with it.
    pub fn attach_inline_to_draft(
        &self,
        draft_id: &str,
        name: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<Option<DraftFile>, StoreError> {
        self.attach(draft_id, name, content_type, data, true)
    }

    fn attach(
        &self,
        draft_id: &str,
        name: &str,
        content_type: &str,
        data: &[u8],
        inline: bool,
    ) -> Result<Option<DraftFile>, StoreError> {
        let connection = self.connection();
        let added = connection.execute(
            "INSERT INTO draft_files (draft_id, name, content_type, size, data)
             SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM drafts WHERE id = ?1 AND state <> 'queued')",
            params![draft_id, name, content_type, data.len() as i64, data],
        )?;
        if added == 0 {
            return Ok(None);
        }
        let id = connection.last_insert_rowid();
        // Made from the file's own number, so no two are alike.
        let content_id = inline.then(|| format!("picture-{id}@brolga"));
        connection.execute(
            "UPDATE draft_files SET content_id = ?2 WHERE id = ?1",
            params![id, content_id],
        )?;
        connection.execute(
            "UPDATE drafts SET updated_at = unixepoch() WHERE id = ?1",
            [draft_id],
        )?;

        Ok(Some(DraftFile {
            id,
            name: name.to_owned(),
            content_type: content_type.to_owned(),
            size: data.len() as i64,
            upload_tried: false,
            uploaded: false,
            content_id,
            attachment_id: None,
        }))
    }

    /// Count as a draft's own a file the server's copy already has, on a
    /// draft written in another mail program, by its id on that copy: a
    /// picture within the body, under the name the body refers to it by,
    /// or a file beside it, with none. It is not sent there again, and
    /// taken off here it is taken off there, as a file put on here would be.
    pub fn adopt_draft_file(
        &self,
        draft_id: &str,
        name: &str,
        content_type: &str,
        data: &[u8],
        content_id: Option<&str>,
        attachment_id: &str,
    ) -> Result<DraftFile, StoreError> {
        let connection = self.connection();
        connection.execute(
            "INSERT INTO draft_files (draft_id, name, content_type, size, data, upload_tried, uploaded, content_id, attachment_id)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, 1, ?6, ?7)",
            params![draft_id, name, content_type, data.len() as i64, data, content_id, attachment_id],
        )?;

        Ok(DraftFile {
            id: connection.last_insert_rowid(),
            name: name.to_owned(),
            content_type: content_type.to_owned(),
            size: data.len() as i64,
            upload_tried: true,
            uploaded: true,
            content_id: content_id.map(str::to_owned),
            attachment_id: Some(attachment_id.to_owned()),
        })
    }

    /// How many pictures kept on the web were left out of a draft written
    /// in another mail program when it was taken up here.
    pub fn draft_web_pictures(&self, draft_id: &str) -> Result<u32, StoreError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT web_pictures FROM drafts WHERE id = ?1",
                [draft_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }

    pub fn set_draft_web_pictures(&self, draft_id: &str, count: u32) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE drafts SET web_pictures = ?2 WHERE id = ?1",
            params![draft_id, count],
        )?;

        Ok(())
    }

    /// One file of a draft, with its contents.
    pub fn draft_file(
        &self,
        draft_id: &str,
        file_id: i64,
    ) -> Result<Option<(DraftFile, Vec<u8>)>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT id, name, content_type, size, upload_tried, uploaded, content_id, attachment_id, data
                 FROM draft_files WHERE draft_id = ?1 AND id = ?2",
                params![draft_id, file_id],
                |row| Ok((read_file(row)?, row.get(8)?)),
            )
            .optional()?)
    }

    /// The files attached to a draft, in the order they were attached.
    pub fn draft_files(&self, draft_id: &str) -> Result<Vec<DraftFile>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT id, name, content_type, size, upload_tried, uploaded, content_id, attachment_id
             FROM draft_files WHERE draft_id = ?1 ORDER BY id",
        )?;
        let files = statement
            .query_map([draft_id], read_file)?
            .collect::<Result<_, _>>()?;

        Ok(files)
    }

    /// How many bytes of files a draft carries.
    pub fn draft_files_size(&self, draft_id: &str) -> Result<i64, StoreError> {
        Ok(self.connection().query_row(
            "SELECT coalesce(sum(size), 0) FROM draft_files WHERE draft_id = ?1",
            [draft_id],
            |row| row.get(0),
        )?)
    }

    pub fn draft_file_data(&self, file_id: i64) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT data FROM draft_files WHERE id = ?1",
                [file_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Take a file off a draft that is not on its way out. If a request to
    /// attach it has gone out, the server's copy of the draft may have it,
    /// and that is noted, so that it is taken off there before the draft
    /// is sent.
    pub fn detach_from_draft(&self, draft_id: &str, file_id: i64) -> Result<bool, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let file: Option<(String, bool, Option<String>)> = transaction
            .query_row(
                "SELECT name, upload_tried, attachment_id FROM draft_files WHERE id = ?2 AND draft_id = ?1
                     AND EXISTS (SELECT 1 FROM drafts WHERE id = ?1 AND state <> 'queued')",
                params![draft_id, file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((name, upload_tried, attachment_id)) = file else {
            return Ok(false);
        };
        if upload_tried {
            transaction.execute(
                "INSERT INTO draft_files_removed (draft_id, attachment_id, name) VALUES (?1, ?2, ?3)",
                params![draft_id, attachment_id, name],
            )?;
        }
        transaction.execute("DELETE FROM draft_files WHERE id = ?1", [file_id])?;
        transaction.commit()?;

        Ok(true)
    }

    /// The files taken off a draft that its copy on the server may still
    /// have, oldest first: each with a number to call it done by, its id
    /// on the server where that is known, and its name.
    pub fn draft_files_removed(
        &self,
        draft_id: &str,
    ) -> Result<Vec<(i64, Option<String>, String)>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT id, attachment_id, name FROM draft_files_removed WHERE draft_id = ?1 ORDER BY id",
        )?;
        let removed = statement
            .query_map([draft_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<_, _>>()?;

        Ok(removed)
    }

    /// A file taken off a draft is off the server's copy of it too.
    pub fn draft_file_removal_done(&self, removal_id: i64) -> Result<(), StoreError> {
        self.connection().execute(
            "DELETE FROM draft_files_removed WHERE id = ?1",
            [removal_id],
        )?;

        Ok(())
    }

    /// The ids of what was attached to the server's copy of a draft before
    /// any file went to it from here. Nothing until that has been looked at.
    pub fn draft_attachments_before(
        &self,
        draft_id: &str,
    ) -> Result<Option<Vec<String>>, StoreError> {
        let kept: Option<Option<String>> = self
            .connection()
            .query_row(
                "SELECT attachments_before FROM drafts WHERE id = ?1",
                [draft_id],
                |row| row.get(0),
            )
            .optional()?;

        Ok(kept
            .flatten()
            .map(|ids| serde_json::from_str(&ids).unwrap_or_default()))
    }

    /// Note what is attached to the server's copy of a draft, before the
    /// first file goes to it from here.
    pub fn set_draft_attachments_before(
        &self,
        draft_id: &str,
        attachment_ids: &[String],
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE drafts SET attachments_before = ?2 WHERE id = ?1",
            params![
                draft_id,
                serde_json::Value::from(attachment_ids.to_vec()).to_string()
            ],
        )?;

        Ok(())
    }

    /// A request to attach the file is about to go out.
    pub fn draft_file_upload_tried(&self, file_id: i64) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE draft_files SET upload_tried = 1 WHERE id = ?1",
            [file_id],
        )?;

        Ok(())
    }

    /// The server has the file, under this id where it said.
    pub fn draft_file_uploaded(
        &self,
        file_id: i64,
        attachment_id: Option<&str>,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE draft_files SET uploaded = 1, attachment_id = ?2 WHERE id = ?1",
            params![file_id, attachment_id],
        )?;

        Ok(())
    }

    /// The server's copy of the draft does not have the file after all. It
    /// is to go there again.
    pub fn draft_file_not_uploaded(&self, file_id: i64) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE draft_files SET uploaded = 0, upload_tried = 0, attachment_id = NULL WHERE id = ?1",
            [file_id],
        )?;

        Ok(())
    }

    /// Throw a draft away. One that is queued is on its way and cannot be.
    pub fn discard_draft(&self, id: &str) -> Result<bool, StoreError> {
        let gone = self.connection().execute(
            "DELETE FROM drafts WHERE id = ?1 AND state <> 'queued'",
            [id],
        )?;

        Ok(gone > 0)
    }

    /// A copy of a draft on the server is no longer wanted, and is not
    /// among the messages here to be moved as one of those is. It is kept
    /// in mind until it has been moved to Deleted Items there.
    pub fn discard_on_server(&self, server_id: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT OR IGNORE INTO draft_discards (server_id) VALUES (?1)",
            [server_id],
        )?;

        Ok(())
    }

    /// The copies of drafts on the server still to be moved to Deleted
    /// Items, each with how many times that has been tried.
    pub fn draft_discards(&self) -> Result<Vec<(String, u32)>, StoreError> {
        let connection = self.connection();
        let mut statement =
            connection.prepare("SELECT server_id, attempts FROM draft_discards ORDER BY rowid")?;
        let discards = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;

        Ok(discards)
    }

    /// An unwanted copy could not be moved this time. It is tried again.
    pub fn draft_discard_waits(&self, server_id: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE draft_discards SET attempts = attempts + 1 WHERE server_id = ?1",
            [server_id],
        )?;

        Ok(())
    }

    /// An unwanted copy is in Deleted Items, or gone, or the server will
    /// not move it: there is nothing more to do about it.
    pub fn draft_discard_done(&self, server_id: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "DELETE FROM draft_discards WHERE server_id = ?1",
            [server_id],
        )?;

        Ok(())
    }

    /// Queue a draft to be sent. From here it is the outbox's.
    pub fn queue_draft(&self, id: &str) -> Result<bool, StoreError> {
        let queued = self.connection().execute(
            "UPDATE drafts SET state = 'queued', error = NULL, updated_at = unixepoch()
             WHERE id = ?1 AND state <> 'queued'",
            [id],
        )?;

        Ok(queued > 0)
    }

    /// Take a queued draft back to be edited, if no request to send it has
    /// gone out yet. Once one has, it may be sent already. It counts as a
    /// change to the draft, so that an outbox with the draft in hand, as it
    /// was when queued, can tell it is no longer that draft's to send.
    pub fn unqueue_draft(&self, id: &str) -> Result<bool, StoreError> {
        let back = self.connection().execute(
            "UPDATE drafts SET state = 'editing', rev = rev + 1 WHERE id = ?1 AND state = 'queued' AND send_tried = 0",
            [id],
        )?;

        Ok(back > 0)
    }

    /// The drafts waiting to be sent, in the order they were queued.
    pub fn queued_drafts(&self) -> Result<Vec<Draft>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM drafts WHERE state = 'queued' ORDER BY updated_at, seq"
        ))?;
        let drafts = statement.query_map([], read)?.collect::<Result<_, _>>()?;

        Ok(drafts)
    }

    /// The drafts whose copy on the server is behind what is here, or that
    /// have none yet. One with nothing in it is not worth a copy.
    pub fn drafts_to_upload(&self) -> Result<Vec<Draft>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM drafts
             WHERE state = 'editing'
               AND (rev <> server_rev
                    -- Its words are as they were, and its files are not.
                    OR EXISTS (SELECT 1 FROM draft_files f WHERE f.draft_id = drafts.id AND NOT f.uploaded)
                    OR EXISTS (SELECT 1 FROM draft_files_removed r WHERE r.draft_id = drafts.id))
               AND (server_id IS NOT NULL OR subject <> '' OR trim(body) <> ''
                    OR to_recipients <> '[]' OR cc_recipients <> '[]' OR bcc_recipients <> '[]'
                    OR EXISTS (SELECT 1 FROM draft_files f WHERE f.draft_id = drafts.id))
             ORDER BY seq"
        ))?;
        let drafts = statement.query_map([], read)?.collect::<Result<_, _>>()?;

        Ok(drafts)
    }

    /// The server's copy is now as the draft was at this count of changes.
    /// Changes made since leave it still to be uploaded.
    pub fn draft_uploaded(&self, id: &str, rev: i64) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE drafts SET server_rev = ?2 WHERE id = ?1",
            params![id, rev],
        )?;

        Ok(())
    }

    /// The draft that is this message on the server, if one is.
    pub fn draft_for_message(&self, server_id: &str) -> Result<Option<Draft>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT {COLUMNS} FROM drafts WHERE server_id = ?1"),
                [server_id],
                read,
            )
            .optional()?)
    }

    /// Take up a draft that was written elsewhere and is on the server, to
    /// carry on with it here. It starts level with the server's copy.
    pub fn adopt_draft(&self, server_id: &str, content: &DraftEdit) -> Result<Draft, StoreError> {
        let draft = self.create_draft("new", None, content)?;
        self.connection().execute(
            "UPDATE drafts SET server_id = ?2, server_rev = rev WHERE id = ?1",
            params![draft.id, server_id],
        )?;

        Ok(self.draft(&draft.id)?.unwrap_or(draft))
    }

    /// The server's copy is gone, and whatever was attached to it with it.
    /// The draft carries on here without one, and its files are to go to
    /// the next copy as if they had gone to none.
    pub fn draft_unlinked(&self, id: &str) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        unlink(&transaction, id)?;
        transaction.commit()?;

        Ok(())
    }

    /// The draft now exists on the server. False if the draft is no longer
    /// here: it was thrown away while its copy was being made.
    pub fn draft_on_server(
        &self,
        id: &str,
        server_id: &str,
        quoted: Option<&str>,
    ) -> Result<bool, StoreError> {
        let linked = self.connection().execute(
            "UPDATE drafts SET server_id = ?2, quoted = ?3 WHERE id = ?1",
            params![id, server_id, quoted],
        )?;

        Ok(linked > 0)
    }

    /// A request to send is about to go out. Written first, so that if the
    /// answer never comes it is known that the message may have been sent.
    ///
    /// `rev` is the draft's count of changes as the outbox has it in hand.
    /// False, and nothing written, if the draft is no longer queued or has
    /// changed since: the send was taken back meanwhile, and must not go.
    pub fn draft_send_tried(&self, id: &str, rev: i64) -> Result<bool, StoreError> {
        let still = self.connection().execute(
            "UPDATE drafts SET send_tried = 1, send_tried_at = unixepoch()
             WHERE id = ?1 AND state = 'queued' AND rev = ?2",
            params![id, rev],
        )?;

        Ok(still > 0)
    }

    /// The request to send did not reach the server, or was turned away
    /// before anything was done with it. It may be made again.
    pub fn draft_send_not_made(&self, id: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE drafts SET send_tried = 0, send_tried_at = NULL WHERE id = ?1",
            [id],
        )?;

        Ok(())
    }

    /// How many seconds ago the request to send a draft went out. Nothing
    /// if none has, or if when was not written down: then it is not known
    /// to be recent.
    pub fn draft_send_tried_ago(&self, id: &str) -> Result<Option<i64>, StoreError> {
        let ago: Option<Option<i64>> = self
            .connection()
            .query_row(
                "SELECT unixepoch() - send_tried_at FROM drafts WHERE id = ?1 AND send_tried = 1",
                [id],
                |row| row.get(0),
            )
            .optional()?;

        Ok(ago.flatten())
    }

    /// What a draft's copy on the server was to be marked with, if one has
    /// been asked for and is not yet known of here.
    pub fn draft_create_marker(&self, id: &str) -> Result<Option<String>, StoreError> {
        let marker: Option<Option<String>> = self
            .connection()
            .query_row(
                "SELECT create_marker FROM drafts WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;

        Ok(marker.flatten())
    }

    /// Note what a draft's copy on the server is about to be marked with,
    /// before it is asked for.
    pub fn set_draft_create_marker(&self, id: &str, marker: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE drafts SET create_marker = ?2 WHERE id = ?1",
            params![id, marker],
        )?;

        Ok(())
    }

    /// It could not be sent this time. It stays queued.
    pub fn draft_waits(&self, id: &str, error: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE drafts SET error = ?2 WHERE id = ?1 AND state = 'queued'",
            params![id, error],
        )?;

        Ok(())
    }

    /// The server will not send it as it is. It goes back to its writer,
    /// with the reason, and nothing of it is lost. One taken back by its
    /// writer already is left as they have it.
    pub fn draft_refused(&self, id: &str, error: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE drafts SET state = 'failed', error = ?2, send_tried = 0 WHERE id = ?1 AND state = 'queued'",
            params![id, error],
        )?;

        Ok(())
    }

    /// The server has sent it. The copy in Sent Items comes with the next sync.
    ///
    /// Only a draft still queued is done with. Should it have been taken
    /// back and be in its writer's hands, what is here may have been
    /// changed since and is kept: false is given, and the draft is parted
    /// from the copy that went, with a word to say that one did.
    pub fn draft_sent(&self, id: &str) -> Result<bool, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let done = transaction.execute(
            "DELETE FROM drafts WHERE id = ?1 AND state = 'queued'",
            [id],
        )?;
        if done == 0 {
            unlink(&transaction, id)?;
            transaction.execute(
                "UPDATE drafts SET error = 'This message was sent as it stood before it was taken back. What is here has not been sent.'
                 WHERE id = ?1",
                [id],
            )?;
        }
        transaction.commit()?;

        Ok(done > 0)
    }
}

/// Part a draft from its copy on the server, and from everything that was
/// known about that copy: which of its files had gone there, what was
/// attached there first, and what was still to be taken off it.
fn unlink(transaction: &rusqlite::Transaction<'_>, id: &str) -> Result<(), StoreError> {
    transaction.execute(
        "UPDATE drafts SET server_id = NULL, quoted = NULL, server_rev = -1, send_tried = 0, send_tried_at = NULL,
                create_marker = NULL, attachments_before = NULL
         WHERE id = ?1",
        [id],
    )?;
    transaction.execute(
        "UPDATE draft_files SET uploaded = 0, upload_tried = 0, attachment_id = NULL WHERE draft_id = ?1",
        [id],
    )?;
    transaction.execute("DELETE FROM draft_files_removed WHERE draft_id = ?1", [id])?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(body: &str) -> DraftEdit {
        DraftEdit {
            subject: Some("Hello".to_owned()),
            body: Some(body.to_owned()),
            ..DraftEdit::default()
        }
    }

    #[test]
    fn a_draft_is_kept_edited_and_discarded() {
        let db = AccountDb::in_memory().unwrap();
        let draft = db.create_draft("new", None, &start("One")).unwrap();

        assert!(db.edit_draft(&draft.id, &start("Two")).unwrap());
        let kept = db.draft(&draft.id).unwrap().unwrap();
        assert_eq!(
            (kept.body.as_str(), kept.subject.as_str()),
            ("Two", "Hello")
        );
        assert_eq!(kept.to_recipients, "[]");
        assert_eq!(db.drafts().unwrap().len(), 1);

        assert!(db.discard_draft(&draft.id).unwrap());
        assert!(db.draft(&draft.id).unwrap().is_none());
    }

    #[test]
    fn a_queued_draft_cannot_be_changed_or_thrown_away_under_the_outbox() {
        let db = AccountDb::in_memory().unwrap();
        let draft = db.create_draft("new", None, &start("One")).unwrap();
        assert!(db.queue_draft(&draft.id).unwrap());

        assert!(!db.edit_draft(&draft.id, &start("Two")).unwrap());
        assert!(!db.discard_draft(&draft.id).unwrap());
        assert!(!db.queue_draft(&draft.id).unwrap());
        assert_eq!(db.queued_drafts().unwrap()[0].body, "One");
    }

    #[test]
    fn a_queued_draft_can_be_taken_back_only_before_a_send_has_gone_out() {
        let db = AccountDb::in_memory().unwrap();
        let draft = db.create_draft("new", None, &start("One")).unwrap();
        db.queue_draft(&draft.id).unwrap();
        assert!(db.unqueue_draft(&draft.id).unwrap());

        db.queue_draft(&draft.id).unwrap();
        let queued = db.draft(&draft.id).unwrap().unwrap();
        assert!(db.draft_send_tried(&draft.id, queued.rev).unwrap());
        assert!(!db.unqueue_draft(&draft.id).unwrap());
    }

    #[test]
    fn a_send_cannot_go_out_for_a_draft_taken_back_since_it_was_picked_up() {
        let db = AccountDb::in_memory().unwrap();
        let draft = db.create_draft("new", None, &start("One")).unwrap();
        db.queue_draft(&draft.id).unwrap();
        let in_hand = db.draft(&draft.id).unwrap().unwrap();

        // Taken back: not queued, so no send may go.
        assert!(db.unqueue_draft(&draft.id).unwrap());
        assert!(!db.draft_send_tried(&draft.id, in_hand.rev).unwrap());
        // Queued again, it is not the draft the outbox had in hand.
        db.queue_draft(&draft.id).unwrap();
        assert!(!db.draft_send_tried(&draft.id, in_hand.rev).unwrap());
        assert!(!db.draft(&draft.id).unwrap().unwrap().send_tried);
    }

    #[test]
    fn a_draft_in_its_writers_hands_is_kept_when_a_copy_of_it_is_found_sent() {
        let db = AccountDb::in_memory().unwrap();
        let draft = db.create_draft("new", None, &start("One")).unwrap();
        db.draft_on_server(&draft.id, "server-1", None).unwrap();
        db.edit_draft(&draft.id, &start("Two")).unwrap();

        assert!(!db.draft_sent(&draft.id).unwrap());

        let kept = db.draft(&draft.id).unwrap().unwrap();
        assert_eq!((kept.body.as_str(), kept.server_id), ("Two", None));
        assert!(kept.error.unwrap().contains("has not been sent"));
    }

    #[test]
    fn a_draft_is_due_for_the_server_when_it_has_changed_since_it_went() {
        let db = AccountDb::in_memory().unwrap();
        let empty = db.create_draft("new", None, &DraftEdit::default()).unwrap();
        let draft = db.create_draft("new", None, &start("One")).unwrap();
        let due = || -> Vec<String> {
            db.drafts_to_upload()
                .unwrap()
                .into_iter()
                .map(|d| d.id)
                .collect()
        };

        // A draft with nothing in it is not worth a copy.
        assert_eq!(due(), std::slice::from_ref(&draft.id));
        assert!(!due().contains(&empty.id));

        db.draft_on_server(&draft.id, "server-1", None).unwrap();
        db.draft_uploaded(&draft.id, 0).unwrap();
        assert!(due().is_empty());

        // Changed while an upload of the older words was on its way: still due.
        db.edit_draft(&draft.id, &start("Two")).unwrap();
        db.edit_draft(&draft.id, &start("Three")).unwrap();
        db.draft_uploaded(&draft.id, 1).unwrap();
        assert_eq!(due(), std::slice::from_ref(&draft.id));
        db.draft_uploaded(&draft.id, 2).unwrap();
        assert!(due().is_empty());

        assert_eq!(
            db.draft_for_message("server-1").unwrap().unwrap().id,
            draft.id
        );
        db.draft_unlinked(&draft.id).unwrap();
        assert!(db.draft_for_message("server-1").unwrap().is_none());
        assert_eq!(due(), [draft.id]);
    }

    #[test]
    fn a_draft_written_elsewhere_is_taken_up_level_with_the_server() {
        let db = AccountDb::in_memory().unwrap();

        let adopted = db.adopt_draft("server-9", &start("From Outlook")).unwrap();

        assert_eq!(adopted.server_id.as_deref(), Some("server-9"));
        assert_eq!(adopted.body, "From Outlook");
        assert!(db.drafts_to_upload().unwrap().is_empty());
    }

    #[test]
    fn files_stay_with_a_draft_and_go_when_it_does() {
        let db = AccountDb::in_memory().unwrap();
        let draft = db.create_draft("new", None, &start("One")).unwrap();

        let file = db
            .attach_to_draft(&draft.id, "notes.txt", "text/plain", b"Some notes.")
            .unwrap()
            .unwrap();
        db.attach_to_draft(&draft.id, "more.txt", "text/plain", b"More.")
            .unwrap();
        assert!(
            db.attach_to_draft("no-such", "x", "text/plain", b"x")
                .unwrap()
                .is_none()
        );

        assert_eq!(db.draft_files(&draft.id).unwrap().len(), 2);
        assert_eq!(db.draft_files_size(&draft.id).unwrap(), 16);
        assert_eq!(
            db.draft_file_data(file.id).unwrap().unwrap(),
            b"Some notes."
        );
        assert!(db.detach_from_draft(&draft.id, file.id).unwrap());
        assert_eq!(db.draft_files(&draft.id).unwrap()[0].name, "more.txt");

        // Once queued the files are the outbox's, like the rest of it.
        db.queue_draft(&draft.id).unwrap();
        assert!(
            db.attach_to_draft(&draft.id, "late.txt", "text/plain", b"x")
                .unwrap()
                .is_none()
        );
        assert!(
            !db.detach_from_draft(&draft.id, db.draft_files(&draft.id).unwrap()[0].id)
                .unwrap()
        );

        db.draft_sent(&draft.id).unwrap();
        assert!(db.draft_files(&draft.id).unwrap().is_empty());
    }

    #[test]
    fn a_picture_for_the_body_gets_a_name_of_its_own_to_be_referred_to_by() {
        let db = AccountDb::in_memory().unwrap();
        let draft = db.create_draft("new", None, &start("One")).unwrap();

        let one = db
            .attach_inline_to_draft(&draft.id, "a.png", "image/png", b"one")
            .unwrap()
            .unwrap();
        let two = db
            .attach_inline_to_draft(&draft.id, "a.png", "image/png", b"two")
            .unwrap()
            .unwrap();
        let plain = db
            .attach_to_draft(&draft.id, "a.png", "image/png", b"x")
            .unwrap()
            .unwrap();

        assert!(one.content_id.is_some() && one.content_id != two.content_id);
        assert_eq!(plain.content_id, None);
        let (file, data) = db.draft_file(&draft.id, two.id).unwrap().unwrap();
        assert_eq!((file.content_id, data), (two.content_id, b"two".to_vec()));
        assert!(db.draft_file("draft-9", two.id).unwrap().is_none());
    }

    #[test]
    fn a_refused_draft_comes_back_whole_to_be_put_right() {
        let db = AccountDb::in_memory().unwrap();
        let draft = db.create_draft("new", None, &start("One")).unwrap();
        db.queue_draft(&draft.id).unwrap();

        db.draft_refused(&draft.id, "No such recipient.").unwrap();

        let back = db.draft(&draft.id).unwrap().unwrap();
        assert_eq!(back.state, "failed");
        assert_eq!(back.error.as_deref(), Some("No such recipient."));
        assert_eq!(back.body, "One");
        assert!(db.queued_drafts().unwrap().is_empty());
        assert!(db.edit_draft(&draft.id, &start("Two")).unwrap());
        assert_eq!(db.draft(&draft.id).unwrap().unwrap().state, "editing");
    }
}
