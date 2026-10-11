//! The outbox: every change a person makes to their mail.
//!
//! A change is made here first and sent afterwards. Making it and queueing
//! it are one transaction, so the mail on screen and the queue of what is
//! owed to the server cannot disagree, whatever happens next. Until a queued
//! change has been sent, a sync that brings down the server's older value
//! does not undo it on screen.

use rusqlite::{OptionalExtension, Transaction, params};

use crate::{AccountDb, StoreError};

/// A change to one property of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageEdit {
    Read(bool),
    /// `notFlagged`, `flagged` or `complete`.
    Flag(String),
    /// To be followed up by this day, `YYYY-MM-DD`, or by no day if empty.
    FlagDue(String),
    /// To the folder with this id.
    Move(String),
    /// These categories and no others, by name.
    Categories(Vec<String>),
}

impl MessageEdit {
    /// The column it changes, which is also the name it is queued under.
    fn kind(&self) -> &'static str {
        match self {
            Self::Read(_) => "is_read",
            Self::Flag(_) => "flag_status",
            Self::FlagDue(_) => "flag_due",
            Self::Move(_) => "folder_id",
            Self::Categories(_) => "categories",
        }
    }

    fn value(&self) -> String {
        match self {
            Self::Read(read) => i64::from(*read).to_string(),
            Self::Flag(status) => status.clone(),
            Self::FlagDue(day) => day.clone(),
            Self::Move(folder) => folder.clone(),
            // Kept as the message keeps them: a JSON array of names.
            Self::Categories(names) => serde_json::Value::from(names.clone()).to_string(),
        }
    }
}

/// A queued change, as the code that sends it needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    pub id: i64,
    /// Made up here, once, and sent with the change wherever the server can
    /// use it to recognise a repeat.
    pub key: String,
    pub message_id: String,
    /// `is_read`, `flag_status`, `folder_id` or `categories`.
    pub kind: String,
    pub value: String,
    /// How many times it has been tried. After the first, a change that is
    /// not safe to repeat blindly is checked for before it is sent again.
    pub attempts: u32,
}

/// A change that the server refused for good, and that was taken back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedOp {
    pub id: i64,
    pub message_id: String,
    pub subject: String,
    pub kind: String,
    pub error: String,
}

/// The columns a queued change may name. Anything else is a bug, and is
/// never put into SQL.
fn column(kind: &str) -> Option<&'static str> {
    match kind {
        "is_read" => Some("is_read"),
        "flag_status" => Some("flag_status"),
        "flag_due" => Some("flag_due"),
        "folder_id" => Some("folder_id"),
        "categories" => Some("categories"),
        _ => None,
    }
}

impl AccountDb {
    /// A name for an action about to be queued, by which to undo it.
    pub fn new_action(&self) -> Result<String, StoreError> {
        Ok(self
            .connection()
            .query_row("SELECT lower(hex(randomblob(8)))", [], |row| row.get(0))?)
    }

    /// Make a change to some messages and queue it for the server, together
    /// or not at all. A message that already is as asked is left out.
    /// `group` names the action, so that it can be undone as one. Gives
    /// back how many messages changed.
    pub fn queue_edit(
        &self,
        group: &str,
        message_ids: &[String],
        edit: &MessageEdit,
    ) -> Result<u32, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        // What was sent long enough ago is of no more use for undo.
        transaction.execute(
            "DELETE FROM ops WHERE state = 'done' AND created_at < unixepoch() - 86400",
            [],
        )?;

        let mut changed = 0;
        for id in message_ids {
            changed += u32::from(queue(&transaction, group, id, edit.kind(), &edit.value())?);
        }
        transaction.commit()?;

        Ok(changed)
    }

    /// Count every waiting change as made. For mail with no server behind
    /// it, where the change made here is all the change there is.
    pub fn settle_queued_ops(&self) -> Result<(), StoreError> {
        self.connection()
            .execute("UPDATE ops SET state = 'done' WHERE state = 'queued'", [])?;

        Ok(())
    }

    /// The changes waiting to be sent, oldest first.
    pub fn queued_ops(&self) -> Result<Vec<Op>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT id, key, message_id, kind, value, attempts FROM ops WHERE state = 'queued' ORDER BY id",
        )?;
        let ops = statement
            .query_map([], |row| {
                Ok(Op {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    message_id: row.get(2)?,
                    kind: row.get(3)?,
                    value: row.get(4)?,
                    attempts: row.get(5)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(ops)
    }

    /// The server has the change.
    pub fn op_sent(&self, id: i64) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE ops SET state = 'done', error = NULL WHERE id = ?1",
            [id],
        )?;

        Ok(())
    }

    /// The change could not be sent this time. It stays queued.
    pub fn op_waits(&self, id: i64, error: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE ops SET attempts = attempts + 1, error = ?2 WHERE id = ?1",
            params![id, error],
        )?;

        Ok(())
    }

    /// The server will not take the change. It is taken back here too, so
    /// the screen shows what the server has, unless a later change to the
    /// same thing is still waiting, which then stands.
    pub fn op_refused(&self, id: i64, error: &str) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let op: Option<(String, String, String)> = transaction
            .query_row(
                "SELECT message_id, kind, before FROM ops WHERE id = ?1 AND state = 'queued'",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((message_id, kind, before)) = op {
            let later: bool = transaction.query_row(
                "SELECT EXISTS (SELECT 1 FROM ops WHERE message_id = ?1 AND kind = ?2 AND state = 'queued' AND id > ?3)",
                params![message_id, kind, id],
                |row| row.get(0),
            )?;
            if !later && let Some(column) = column(&kind) {
                set(&transaction, column, &message_id, &before)?;
            }
            transaction.execute(
                "UPDATE ops SET state = 'failed', error = ?2 WHERE id = ?1",
                params![id, error],
            )?;
        }
        transaction.commit()?;

        Ok(())
    }

    /// How many changes are waiting, and those that were refused.
    pub fn outbox(&self) -> Result<(u32, Vec<FailedOp>), StoreError> {
        let connection = self.connection();
        let queued =
            connection.query_row("SELECT count(*) FROM ops WHERE state = 'queued'", [], |r| {
                r.get(0)
            })?;
        let mut statement = connection.prepare(
            "SELECT o.id, o.message_id, coalesce(m.subject, ''), o.kind, coalesce(o.error, '')
             FROM ops o LEFT JOIN messages m ON m.id = o.message_id
             WHERE o.state = 'failed' ORDER BY o.id",
        )?;
        let failed = statement
            .query_map([], |row| {
                Ok(FailedOp {
                    id: row.get(0)?,
                    message_id: row.get(1)?,
                    subject: row.get(2)?,
                    kind: row.get(3)?,
                    error: row.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok((queued, failed))
    }

    /// Forget the refused changes, once a person has been told of them.
    pub fn dismiss_failed_ops(&self) -> Result<(), StoreError> {
        self.connection()
            .execute("DELETE FROM ops WHERE state = 'failed'", [])?;

        Ok(())
    }

    /// Take an action back: each message it changed goes back to what it
    /// was, as a new queued change, unless it has been changed again since.
    /// Gives back how many messages went back.
    pub fn undo(&self, group: &str, undo_group: &str) -> Result<u32, StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let ops: Vec<(String, String, String, String)> = {
            let mut statement = transaction.prepare(
                "SELECT message_id, kind, value, before FROM ops
                 WHERE group_id = ?1 AND state IN ('queued', 'done') ORDER BY id DESC",
            )?;
            statement
                .query_map([group], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect::<Result<_, _>>()?
        };

        let mut undone = 0;
        for (message_id, kind, value, before) in ops {
            let Some(column) = column(&kind) else {
                continue;
            };
            let now: Option<String> = transaction
                .query_row(
                    &format!("SELECT CAST({column} AS TEXT) FROM messages WHERE id = ?1"),
                    [&message_id],
                    |row| row.get(0),
                )
                .optional()?;
            if now.as_deref() == Some(value.as_str()) {
                undone += u32::from(queue(
                    &transaction,
                    undo_group,
                    &message_id,
                    &kind,
                    &before,
                )?);
            }
        }
        transaction.commit()?;

        Ok(undone)
    }
}

/// Change one property of one message and queue the change. False if there
/// is no such message or it already is as asked.
fn queue(
    transaction: &Transaction<'_>,
    group: &str,
    message_id: &str,
    kind: &str,
    value: &str,
) -> Result<bool, StoreError> {
    let Some(column) = column(kind) else {
        return Ok(false);
    };
    let before: Option<String> = transaction
        .query_row(
            &format!("SELECT CAST({column} AS TEXT) FROM messages WHERE id = ?1"),
            [message_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(before) = before else {
        return Ok(false);
    };
    if before == value {
        return Ok(false);
    }

    set(transaction, column, message_id, value)?;
    transaction.execute(
        "INSERT INTO ops (key, group_id, message_id, kind, value, before)
         VALUES (lower(hex(randomblob(16))), ?1, ?2, ?3, ?4, ?5)",
        params![group, message_id, kind, value, before],
    )?;

    Ok(true)
}

/// Give a message's property a value. The folder's count of unread
/// messages is the server's, and follows only at the next sync, so it is
/// kept in step here for a change of read state.
fn set(
    transaction: &Transaction<'_>,
    column: &str,
    message_id: &str,
    value: &str,
) -> Result<(), StoreError> {
    if column == "folder_id" {
        // The message leaves one folder's counts and joins the other's.
        for (folder, by) in [
            ("(SELECT folder_id FROM messages WHERE id = ?1)", -1),
            ("?2", 1),
        ] {
            transaction.execute(
                &format!(
                    "UPDATE folders SET
                         total = max(0, total + {by}),
                         unread = max(0, unread + {by} * (SELECT NOT is_read FROM messages WHERE id = ?1))
                     WHERE id = {folder} AND EXISTS (SELECT 1 FROM messages WHERE id = ?1 AND folder_id <> ?2)"
                ),
                params![message_id, value],
            )?;
        }
    }
    if column == "is_read" {
        transaction.execute(
            "UPDATE folders SET unread = max(0, unread + CASE WHEN ?2 = '1' THEN -1 ELSE 1 END)
             WHERE id = (SELECT folder_id FROM messages WHERE id = ?1 AND CAST(is_read AS TEXT) <> ?2)",
            params![message_id, value],
        )?;
    }
    transaction.execute(
        &format!("UPDATE messages SET {column} = ?2 WHERE id = ?1"),
        params![message_id, value],
    )?;

    Ok(())
}

/// After a sync has written the server's values for some messages, put back
/// on top of them whatever is still waiting to be sent, so a change made
/// here is not seen to flick back before it reaches the server.
pub(crate) fn keep_queued_edits(
    transaction: &Transaction<'_>,
    message_ids: &[&str],
) -> Result<(), StoreError> {
    let any: bool = transaction.query_row(
        "SELECT EXISTS (SELECT 1 FROM ops WHERE state = 'queued')",
        [],
        |row| row.get(0),
    )?;
    if !any {
        return Ok(());
    }

    for kind in [
        "is_read",
        "flag_status",
        "flag_due",
        "folder_id",
        "categories",
    ] {
        let Some(column) = column(kind) else {
            continue;
        };
        let mut statement = transaction.prepare(&format!(
            "UPDATE messages SET {column} = o.value
             FROM (SELECT value FROM ops WHERE message_id = ?1 AND kind = ?2 AND state = 'queued'
                   ORDER BY id DESC LIMIT 1) o
             WHERE messages.id = ?1"
        ))?;
        for id in message_ids {
            statement.execute(params![id, kind])?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MessageChange, Round};

    fn db() -> AccountDb {
        let db = AccountDb::in_memory().unwrap();
        db.apply_messages(
            "inbox",
            &[server("1", false), server("2", false)],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();
        db
    }

    fn server(id: &str, read: bool) -> MessageChange {
        MessageChange {
            id: id.to_owned(),
            subject: Some(format!("Message {id}")),
            is_read: Some(read),
            flag_status: Some("notFlagged".to_owned()),
            ..MessageChange::default()
        }
    }

    fn read(db: &AccountDb, id: &str) -> bool {
        db.message(id).unwrap().unwrap().is_read
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn a_change_is_made_and_queued_together_and_only_where_it_changes_something() {
        let db = db();

        let changed = db
            .queue_edit("a", &ids(&["1", "2", "no-such"]), &MessageEdit::Read(true))
            .unwrap();
        let again = db
            .queue_edit("b", &ids(&["1"]), &MessageEdit::Read(true))
            .unwrap();

        assert_eq!((changed, again), (2, 0));
        assert!(read(&db, "1") && read(&db, "2"));
        let queued = db.queued_ops().unwrap();
        assert_eq!(queued.len(), 2);
        assert_eq!(
            (queued[0].kind.as_str(), queued[0].value.as_str()),
            ("is_read", "1")
        );
        assert_ne!(queued[0].key, queued[1].key);
    }

    #[test]
    fn the_folders_unread_count_follows_a_change_of_read_state() {
        let db = db();
        db.apply_folders(
            &[crate::FolderChange {
                id: "inbox".to_owned(),
                unread: Some(2),
                ..crate::FolderChange::default()
            }],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();
        let unread = || db.folders().unwrap()[0].unread;

        db.queue_edit("a", &ids(&["1", "2"]), &MessageEdit::Read(true))
            .unwrap();
        assert_eq!(unread(), 0);

        db.op_refused(db.queued_ops().unwrap()[0].id, "No.")
            .unwrap();
        assert_eq!(unread(), 1);

        db.undo("a", "b").unwrap();
        assert_eq!(unread(), 2);
    }

    #[test]
    fn a_move_changes_both_folders_counts_and_is_not_lost_to_a_sync_of_either() {
        let db = db();
        let folder = |id: &str, total: i64| crate::FolderChange {
            id: id.to_owned(),
            name: Some(id.to_owned()),
            unread: Some(total),
            total: Some(total),
            ..crate::FolderChange::default()
        };
        db.apply_folders(
            &[folder("archive", 0), folder("inbox", 2)],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();
        let counts = || -> Vec<(String, i64, i64)> {
            db.folders()
                .unwrap()
                .into_iter()
                .map(|f| (f.id, f.total, f.unread))
                .collect()
        };

        db.queue_edit("a", &ids(&["1"]), &MessageEdit::Move("archive".to_owned()))
            .unwrap();
        assert_eq!(db.message("1").unwrap().unwrap().folder_id, "archive");
        assert_eq!(
            counts(),
            [("archive".to_owned(), 1, 1), ("inbox".to_owned(), 1, 1)]
        );

        // The server still has it in the inbox, and a full pass over the
        // archive has never heard of it. Neither may lose it.
        db.apply_messages("inbox", &[server("1", false)], &[], "next", Round::Finished)
            .unwrap();
        db.restart_messages_sync("archive").unwrap();
        db.apply_messages("archive", &[], &[], "next", Round::Finished)
            .unwrap();
        assert_eq!(db.message("1").unwrap().unwrap().folder_id, "archive");

        db.undo("a", "b").unwrap();
        assert_eq!(db.message("1").unwrap().unwrap().folder_id, "inbox");
        assert_eq!(
            counts(),
            [("archive".to_owned(), 0, 0), ("inbox".to_owned(), 2, 2)]
        );
    }

    #[test]
    fn a_sync_does_not_take_back_a_change_that_is_still_waiting() {
        let db = db();
        db.queue_edit("a", &ids(&["1"]), &MessageEdit::Read(true))
            .unwrap();

        // The server has not heard yet, and says unread.
        db.apply_messages("inbox", &[server("1", false)], &[], "next", Round::Finished)
            .unwrap();
        assert!(read(&db, "1"));

        // Once it is sent the server's word stands again.
        db.op_sent(db.queued_ops().unwrap()[0].id).unwrap();
        db.apply_messages("inbox", &[server("1", false)], &[], "next", Round::Finished)
            .unwrap();
        assert!(!read(&db, "1"));
    }

    #[test]
    fn a_refused_change_is_taken_back_and_reported() {
        let db = db();
        db.queue_edit("a", &ids(&["1"]), &MessageEdit::Read(true))
            .unwrap();
        let op = db.queued_ops().unwrap()[0].id;

        db.op_refused(op, "No.").unwrap();

        assert!(!read(&db, "1"));
        let (queued, failed) = db.outbox().unwrap();
        assert_eq!(queued, 0);
        assert_eq!(failed[0].subject, "Message 1");
        assert_eq!(failed[0].error, "No.");
        db.dismiss_failed_ops().unwrap();
        assert!(db.outbox().unwrap().1.is_empty());
    }

    #[test]
    fn a_refused_change_does_not_take_back_a_later_one_to_the_same_thing() {
        let db = db();
        db.queue_edit("a", &ids(&["1"]), &MessageEdit::Read(true))
            .unwrap();
        db.queue_edit("b", &ids(&["1"]), &MessageEdit::Read(false))
            .unwrap();
        db.queue_edit("c", &ids(&["1"]), &MessageEdit::Read(true))
            .unwrap();
        let first = db.queued_ops().unwrap()[0].id;

        db.op_refused(first, "No.").unwrap();

        assert!(read(&db, "1"));
        assert_eq!(db.queued_ops().unwrap().len(), 2);
    }

    #[test]
    fn an_action_is_undone_as_one_unless_a_message_has_changed_since() {
        let db = db();
        db.queue_edit(
            "a",
            &ids(&["1", "2"]),
            &MessageEdit::Flag("flagged".to_owned()),
        )
        .unwrap();
        // Message 2 is then marked complete, which undoing the flag must not lose.
        db.queue_edit("b", &ids(&["2"]), &MessageEdit::Flag("complete".to_owned()))
            .unwrap();

        let undone = db.undo("a", "undo-a").unwrap();

        assert_eq!(undone, 1);
        assert_eq!(db.message("1").unwrap().unwrap().flag_status, "notFlagged");
        assert_eq!(db.message("2").unwrap().unwrap().flag_status, "complete");
        assert_eq!(db.queued_ops().unwrap().len(), 4);
    }
}
