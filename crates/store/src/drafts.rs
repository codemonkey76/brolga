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
    /// The words as typed, plain.
    pub body: String,
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
}

/// What may be changed about a draft. `None` leaves a part as it is.
#[derive(Debug, Clone, Default)]
pub struct DraftEdit {
    pub to_recipients: Option<String>,
    pub cc_recipients: Option<String>,
    pub bcc_recipients: Option<String>,
    pub subject: Option<String>,
    pub body: Option<String>,
}

const COLUMNS: &str = "id, kind, source_id, to_recipients, cc_recipients, bcc_recipients, subject, body, state, \
                       error, updated_at, server_id, quoted, send_tried";

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
            "INSERT INTO drafts (kind, source_id, to_recipients, cc_recipients, bcc_recipients, subject, body)
             VALUES (?1, ?2, coalesce(?3, '[]'), coalesce(?4, '[]'), coalesce(?5, '[]'), coalesce(?6, ''), coalesce(?7, ''))",
            params![
                kind,
                source_id,
                start.to_recipients,
                start.cc_recipients,
                start.bcc_recipients,
                start.subject,
                start.body
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
    pub fn edit_draft(&self, id: &str, edit: &DraftEdit) -> Result<bool, StoreError> {
        let changed = self.connection().execute(
            "UPDATE drafts SET
                 to_recipients = coalesce(?2, to_recipients),
                 cc_recipients = coalesce(?3, cc_recipients),
                 bcc_recipients = coalesce(?4, bcc_recipients),
                 subject = coalesce(?5, subject),
                 body = coalesce(?6, body),
                 state = 'editing', error = NULL, updated_at = unixepoch()
             WHERE id = ?1 AND state <> 'queued'",
            params![
                id,
                edit.to_recipients,
                edit.cc_recipients,
                edit.bcc_recipients,
                edit.subject,
                edit.body
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
        let connection = self.connection();
        let added = connection.execute(
            "INSERT INTO draft_files (draft_id, name, content_type, size, data)
             SELECT ?1, ?2, ?3, ?4, ?5 WHERE EXISTS (SELECT 1 FROM drafts WHERE id = ?1 AND state <> 'queued')",
            params![draft_id, name, content_type, data.len() as i64, data],
        )?;
        if added == 0 {
            return Ok(None);
        }
        connection.execute(
            "UPDATE drafts SET updated_at = unixepoch() WHERE id = ?1",
            [draft_id],
        )?;

        Ok(Some(DraftFile {
            id: connection.last_insert_rowid(),
            name: name.to_owned(),
            content_type: content_type.to_owned(),
            size: data.len() as i64,
            upload_tried: false,
            uploaded: false,
        }))
    }

    /// The files attached to a draft, in the order they were attached.
    pub fn draft_files(&self, draft_id: &str) -> Result<Vec<DraftFile>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT id, name, content_type, size, upload_tried, uploaded FROM draft_files WHERE draft_id = ?1 ORDER BY id",
        )?;
        let files = statement
            .query_map([draft_id], |row| {
                Ok(DraftFile {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    content_type: row.get(2)?,
                    size: row.get(3)?,
                    upload_tried: row.get(4)?,
                    uploaded: row.get(5)?,
                })
            })?
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

    /// Take a file off a draft that is not on its way out.
    pub fn detach_from_draft(&self, draft_id: &str, file_id: i64) -> Result<bool, StoreError> {
        let gone = self.connection().execute(
            "DELETE FROM draft_files WHERE id = ?2 AND draft_id = ?1
                 AND EXISTS (SELECT 1 FROM drafts WHERE id = ?1 AND state <> 'queued')",
            params![draft_id, file_id],
        )?;

        Ok(gone > 0)
    }

    /// A request to attach the file is about to go out.
    pub fn draft_file_upload_tried(&self, file_id: i64) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE draft_files SET upload_tried = 1 WHERE id = ?1",
            [file_id],
        )?;

        Ok(())
    }

    /// The server has the file.
    pub fn draft_file_uploaded(&self, file_id: i64) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE draft_files SET uploaded = 1 WHERE id = ?1",
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
    /// gone out yet. Once one has, it may be sent already.
    pub fn unqueue_draft(&self, id: &str) -> Result<bool, StoreError> {
        let back = self.connection().execute(
            "UPDATE drafts SET state = 'editing' WHERE id = ?1 AND state = 'queued' AND send_tried = 0",
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

    /// The draft now exists on the server.
    pub fn draft_on_server(
        &self,
        id: &str,
        server_id: &str,
        quoted: Option<&str>,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE drafts SET server_id = ?2, quoted = ?3 WHERE id = ?1",
            params![id, server_id, quoted],
        )?;

        Ok(())
    }

    /// A request to send is about to go out. Written first, so that if the
    /// answer never comes it is known that the message may have been sent.
    pub fn draft_send_tried(&self, id: &str) -> Result<(), StoreError> {
        self.connection()
            .execute("UPDATE drafts SET send_tried = 1 WHERE id = ?1", [id])?;

        Ok(())
    }

    /// It could not be sent this time. It stays queued.
    pub fn draft_waits(&self, id: &str, error: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE drafts SET error = ?2 WHERE id = ?1",
            params![id, error],
        )?;

        Ok(())
    }

    /// The server will not send it as it is. It goes back to its writer,
    /// with the reason, and nothing of it is lost.
    pub fn draft_refused(&self, id: &str, error: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE drafts SET state = 'failed', error = ?2, send_tried = 0 WHERE id = ?1",
            params![id, error],
        )?;

        Ok(())
    }

    /// The server has sent it. The copy in Sent Items comes with the next sync.
    pub fn draft_sent(&self, id: &str) -> Result<(), StoreError> {
        self.connection()
            .execute("DELETE FROM drafts WHERE id = ?1", [id])?;

        Ok(())
    }
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
        db.draft_send_tried(&draft.id).unwrap();
        assert!(!db.unqueue_draft(&draft.id).unwrap());
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
