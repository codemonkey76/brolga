//! Runs of the mail instructions over mail already in the mailbox, and
//! each message in them (docs/spec-mail-instructions.md, "Running over
//! existing mail"). Everything a run has decided or done is kept here, so
//! that it carries on where it was after a restart.

use rusqlite::{OptionalExtension, params};

use crate::{AccountDb, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRow {
    pub id: String,
    pub folder_id: String,
    pub subfolders: bool,
    pub received_from: Option<String>,
    pub received_to: Option<String>,
    /// previewing, previewed, applying, applied, cancelled or undone.
    pub state: String,
    pub provider: String,
    pub model: String,
    pub list_version: i64,
    pub created_at: i64,
    pub applied_at: Option<i64>,
    pub error: String,
    /// The instructions' making of To Do tasks is part of it.
    pub include_tasks: bool,
    /// The instructions' calls to outside tools are part of it, and are
    /// made when it is applied: applying it is the approval.
    pub include_tools: bool,
}

/// A message in a run, with what the list shows of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunItemRow {
    pub message_id: String,
    pub position: i64,
    /// pending, decided (something to do), nothing (nothing to do), failed,
    /// applied, skipped or unapplied.
    pub state: String,
    pub snapshot: String,
    /// JSON: what is to be done.
    pub decisions: String,
    /// JSON: what the model chose and the instructions do not allow.
    pub refused: String,
    /// JSON: what undoes what was done.
    pub undo_ids: String,
    pub error: String,
    pub subject: String,
    pub from_name: String,
    pub from_address: String,
    pub received_at: String,
}

fn read_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        subfolders: row.get(2)?,
        received_from: row.get(3)?,
        received_to: row.get(4)?,
        state: row.get(5)?,
        provider: row.get(6)?,
        model: row.get(7)?,
        list_version: row.get(8)?,
        created_at: row.get(9)?,
        applied_at: row.get(10)?,
        error: row.get(11)?,
        include_tasks: row.get(12)?,
        include_tools: row.get(13)?,
    })
}

const RUN_COLUMNS: &str = "id, folder_id, subfolders, received_from, received_to, state, provider, model, \
     list_version, created_at, applied_at, error, include_tasks, include_tools";

impl AccountDb {
    /// The messages in these folders, oldest first, received within the
    /// range where one is given (`YYYY-MM-DD` or a full time, inclusive of
    /// the day `to` names).
    pub fn messages_for_run(
        &self,
        folder_ids: &[String],
        from: Option<&str>,
        to: Option<&str>,
    ) -> Result<Vec<String>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT id FROM messages
             WHERE folder_id IN (SELECT value FROM json_each(?1)) AND is_draft = 0
               AND (?2 IS NULL OR received_at >= ?2)
               AND (?3 IS NULL OR substr(received_at, 1, length(?3)) <= ?3)
             ORDER BY received_at, id",
        )?;
        let folders = serde_json::Value::from(folder_ids.to_vec()).to_string();
        let ids = statement
            .query_map(params![folders, from, to], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        Ok(ids)
    }

    /// Start a run over these messages, to be previewed.
    #[allow(clippy::too_many_arguments)]
    pub fn create_run(
        &self,
        id: &str,
        folder_id: &str,
        subfolders: bool,
        from: Option<&str>,
        to: Option<&str>,
        provider: &str,
        model: &str,
        list_version: i64,
        include_tasks: bool,
        include_tools: bool,
        message_ids: &[String],
    ) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO instruction_runs (id, folder_id, subfolders, received_from, received_to, state, provider, model, list_version, include_tasks, include_tools)
             VALUES (?1, ?2, ?3, ?4, ?5, 'previewing', ?6, ?7, ?8, ?9, ?10)",
            params![id, folder_id, subfolders, from, to, provider, model, list_version, include_tasks, include_tools],
        )?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO instruction_run_items (run_id, message_id, position) VALUES (?1, ?2, ?3)",
            )?;
            for (position, message_id) in message_ids.iter().enumerate() {
                insert.execute(params![
                    id,
                    message_id,
                    i64::try_from(position).unwrap_or(i64::MAX)
                ])?;
            }
        }
        transaction.commit()?;

        Ok(())
    }

    pub fn run(&self, id: &str) -> Result<Option<RunRow>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT {RUN_COLUMNS} FROM instruction_runs WHERE id = ?1"),
                [id],
                read_run,
            )
            .optional()?)
    }

    /// The latest run, whatever has become of it.
    pub fn latest_run(&self) -> Result<Option<RunRow>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT {RUN_COLUMNS} FROM instruction_runs ORDER BY created_at DESC, rowid DESC LIMIT 1"),
                [],
                read_run,
            )
            .optional()?)
    }

    /// The run being previewed or applied, if one is.
    pub fn active_run(&self) -> Result<Option<RunRow>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT {RUN_COLUMNS} FROM instruction_runs WHERE state IN ('previewing', 'applying') LIMIT 1"),
                [],
                read_run,
            )
            .optional()?)
    }

    pub fn set_run_state(&self, id: &str, state: &str, error: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE instruction_runs SET state = ?2, error = ?3,
                 applied_at = CASE WHEN ?2 = 'applied' THEN unixepoch() ELSE applied_at END
             WHERE id = ?1",
            params![id, state, error],
        )?;

        Ok(())
    }

    /// The next messages in a run that stand in this state, in order.
    pub fn run_items_in(
        &self,
        run_id: &str,
        state: &str,
        limit: u32,
    ) -> Result<Vec<String>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT message_id FROM instruction_run_items WHERE run_id = ?1 AND state = ?2 ORDER BY position LIMIT ?3",
        )?;
        let ids = statement
            .query_map(params![run_id, state, limit], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        Ok(ids)
    }

    /// What the model decided for a message in a run, and how the message
    /// stood when it did.
    pub fn run_item_decided(
        &self,
        run_id: &str,
        message_id: &str,
        snapshot: &str,
        decisions: &str,
        refused: &str,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE instruction_run_items SET snapshot = ?3, decisions = ?4, refused = ?5, error = '',
                 state = CASE WHEN ?4 = '[]' THEN 'nothing' ELSE 'decided' END
             WHERE run_id = ?1 AND message_id = ?2",
            params![run_id, message_id, snapshot, decisions, refused],
        )?;

        Ok(())
    }

    /// A message's place in a run: `failed`, `skipped`, `applied` or
    /// `unapplied`, with why, and what undoes what was done.
    pub fn set_run_item(
        &self,
        run_id: &str,
        message_id: &str,
        state: &str,
        error: &str,
        undo_ids: &str,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE instruction_run_items SET state = ?3, error = ?4, undo_ids = ?5 WHERE run_id = ?1 AND message_id = ?2",
            params![run_id, message_id, state, error, undo_ids],
        )?;

        Ok(())
    }

    pub fn run_item(
        &self,
        run_id: &str,
        message_id: &str,
    ) -> Result<Option<RunItemRow>, StoreError> {
        Ok(self
            .run_items_where(run_id, "AND i.message_id = ?4", message_id, 0, 1)?
            .into_iter()
            .next())
    }

    /// A page of a run's messages in order: all of them, or those that
    /// stand in one state, or (`changes`) those with something to be done.
    pub fn run_items(
        &self,
        run_id: &str,
        filter: Option<&str>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<RunItemRow>, StoreError> {
        match filter {
            None | Some("all") => self.run_items_where(run_id, "", "", offset, limit),
            Some("changes") => self.run_items_where(
                run_id,
                "AND i.decisions != '[]' AND ?4 = ?4",
                "",
                offset,
                limit,
            ),
            Some(state) => self.run_items_where(run_id, "AND i.state = ?4", state, offset, limit),
        }
    }

    fn run_items_where(
        &self,
        run_id: &str,
        clause: &str,
        given: &str,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<RunItemRow>, StoreError> {
        let connection = self.connection();
        let given_clause = if clause.contains("?4") {
            clause.to_owned()
        } else {
            format!("{clause} AND ?4 = ?4")
        };
        let mut statement = connection.prepare(&format!(
            "SELECT i.message_id, i.position, i.state, i.snapshot, i.decisions, i.refused, i.undo_ids, i.error,
                    coalesce(m.subject, ''), coalesce(m.from_name, ''), coalesce(m.from_address, ''), coalesce(m.received_at, '')
             FROM instruction_run_items i LEFT JOIN messages m ON m.id = i.message_id
             WHERE i.run_id = ?1 {given_clause}
             ORDER BY i.position LIMIT ?3 OFFSET ?2"
        ))?;
        let rows = statement
            .query_map(params![run_id, offset, limit, given], |row| {
                Ok(RunItemRow {
                    message_id: row.get(0)?,
                    position: row.get(1)?,
                    state: row.get(2)?,
                    snapshot: row.get(3)?,
                    decisions: row.get(4)?,
                    refused: row.get(5)?,
                    undo_ids: row.get(6)?,
                    error: row.get(7)?,
                    subject: row.get(8)?,
                    from_name: row.get(9)?,
                    from_address: row.get(10)?,
                    received_at: row.get(11)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }

    /// How many of a run's messages stand in each state.
    pub fn run_counts(&self, run_id: &str) -> Result<Vec<(String, u32)>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT state, count(*) FROM instruction_run_items WHERE run_id = ?1 GROUP BY state ORDER BY state",
        )?;
        let counts = statement
            .query_map([run_id], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;

        Ok(counts)
    }

    /// Every decision a run holds, as JSON, one per message, for totals.
    pub fn run_decisions(&self, run_id: &str) -> Result<Vec<String>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT decisions FROM instruction_run_items WHERE run_id = ?1 AND state != 'pending' AND state != 'failed'",
        )?;
        let all = statement
            .query_map([run_id], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        Ok(all)
    }

    /// What undoes everything a run did, by message.
    pub fn run_undo_ids(&self, run_id: &str) -> Result<Vec<String>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT undo_ids FROM instruction_run_items WHERE run_id = ?1 AND undo_ids != '[]' ORDER BY position DESC",
        )?;
        let lists: Vec<String> = statement
            .query_map([run_id], |row| row.get(0))?
            .collect::<Result<_, _>>()?;

        Ok(lists
            .iter()
            .filter_map(|list| serde_json::from_str::<Vec<String>>(list).ok())
            .flatten()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::{MessageChange, Round};

    fn db() -> AccountDb {
        let db = AccountDb::in_memory().unwrap();
        let message = |id: &str, at: &str| MessageChange {
            id: id.to_owned(),
            subject: Some(format!("Message {id}")),
            received_at: Some(at.to_owned()),
            ..MessageChange::default()
        };
        db.apply_messages(
            "inbox",
            &[
                message("a", "2026-01-02T00:00:00Z"),
                message("b", "2026-03-02T09:00:00Z"),
            ],
            &[],
            "n",
            Round::Finished,
        )
        .unwrap();
        db.apply_messages(
            "archive",
            &[message("c", "2026-02-01T00:00:00Z")],
            &[],
            "n",
            Round::Finished,
        )
        .unwrap();
        db
    }

    #[test]
    fn a_runs_messages_are_chosen_by_folder_and_day_oldest_first() {
        let db = db();
        let both = ["inbox".to_owned(), "archive".to_owned()];
        assert_eq!(
            db.messages_for_run(&both, None, None).unwrap(),
            ["a", "c", "b"]
        );
        assert_eq!(
            db.messages_for_run(&both, Some("2026-02-01"), Some("2026-03-02"))
                .unwrap(),
            ["c", "b"]
        );
        assert_eq!(
            db.messages_for_run(&both, None, Some("2026-03-01"))
                .unwrap(),
            ["a", "c"]
        );
    }

    #[test]
    fn a_run_keeps_where_each_message_stands() {
        let db = db();
        let ids = ["a".to_owned(), "b".to_owned()];
        db.create_run(
            "r1", "inbox", false, None, None, "ollama", "llama", 3, false, false, &ids,
        )
        .unwrap();
        assert_eq!(db.active_run().unwrap().unwrap().state, "previewing");
        assert_eq!(db.run_items_in("r1", "pending", 10).unwrap(), ids);

        db.run_item_decided("r1", "a", "{}", r#"[{"action":"move"}]"#, "[]")
            .unwrap();
        db.run_item_decided("r1", "b", "{}", "[]", "[]").unwrap();
        assert_eq!(db.run_items("r1", Some("changes"), 0, 10).unwrap().len(), 1);
        assert_eq!(
            db.run_items("r1", None, 0, 10).unwrap()[0].subject,
            "Message a"
        );

        db.set_run_item("r1", "a", "applied", "", r#"["u1","u2"]"#)
            .unwrap();
        db.set_run_state("r1", "applied", "").unwrap();
        assert!(db.active_run().unwrap().is_none());
        assert!(db.run("r1").unwrap().unwrap().applied_at.is_some());
        assert_eq!(db.run_undo_ids("r1").unwrap(), ["u1", "u2"]);
        assert_eq!(
            db.run_counts("r1").unwrap(),
            [("applied".to_owned(), 1), ("nothing".to_owned(), 1)]
        );
    }
}
