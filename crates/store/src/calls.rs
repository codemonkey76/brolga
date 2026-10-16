//! Calls to outside tools the mail instructions make, as the processing
//! device keeps them: each written before it is made, with the call exactly
//! as it was put up for approval (docs/spec-mail-instructions.md, "Outside
//! tools").

use rusqlite::{OptionalExtension, params};

use crate::{AccountDb, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallRow {
    pub id: String,
    /// The item in the mailbox it waits for approval on, if it does.
    pub approval_id: String,
    pub message_id: String,
    pub server_id: String,
    pub tool: String,
    /// JSON: the call, as put up.
    pub call: String,
    pub state: String,
    pub attempts: u32,
    pub next_try: i64,
    pub outcome: String,
    pub run_id: Option<String>,
    pub changed_at: i64,
    /// JSON: what each step made so far returned.
    pub progress: String,
}

const COLUMNS: &str = "id, approval_id, message_id, server_id, tool, call, state, attempts, next_try, outcome, run_id, changed_at, progress";

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolCallRow> {
    Ok(ToolCallRow {
        id: row.get(0)?,
        approval_id: row.get(1)?,
        message_id: row.get(2)?,
        server_id: row.get(3)?,
        tool: row.get(4)?,
        call: row.get(5)?,
        state: row.get(6)?,
        attempts: row.get(7)?,
        next_try: row.get(8)?,
        outcome: row.get(9)?,
        run_id: row.get(10)?,
        changed_at: row.get(11)?,
        progress: row.get(12)?,
    })
}

impl AccountDb {
    pub fn add_tool_call(&self, call: &ToolCallRow) -> Result<(), StoreError> {
        self.connection().execute(
            &format!("INSERT INTO tool_calls ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, unixepoch(), ?12)"),
            params![
                call.id, call.approval_id, call.message_id, call.server_id, call.tool, call.call, call.state,
                call.attempts, call.next_try, call.outcome, call.run_id, call.progress
            ],
        )?;
        Ok(())
    }

    pub fn tool_call(&self, id: &str) -> Result<Option<ToolCallRow>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT {COLUMNS} FROM tool_calls WHERE id = ?1"),
                [id],
                read,
            )
            .optional()?)
    }

    pub fn tool_call_by_approval(
        &self,
        approval_id: &str,
    ) -> Result<Option<ToolCallRow>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT {COLUMNS} FROM tool_calls WHERE approval_id = ?1"),
                [approval_id],
                read,
            )
            .optional()?)
    }

    pub fn tool_calls_in(&self, state: &str) -> Result<Vec<ToolCallRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM tool_calls WHERE state = ?1 ORDER BY changed_at, id"
        ))?;
        let rows = statement
            .query_map([state], read)?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }

    /// About to be made: written first, and counted.
    pub fn tool_call_calling(&self, id: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE tool_calls SET state = 'calling', attempts = attempts + 1, changed_at = unixepoch() WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    pub fn set_tool_call(
        &self,
        id: &str,
        state: &str,
        outcome: &str,
        next_try: i64,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE tool_calls SET state = ?2, outcome = ?3, next_try = ?4, changed_at = unixepoch() WHERE id = ?1",
            params![id, state, outcome, next_try],
        )?;
        Ok(())
    }

    /// What the steps made so far returned: written as each is made.
    pub fn set_tool_call_progress(&self, id: &str, progress: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE tool_calls SET progress = ?2, changed_at = unixepoch() WHERE id = ?1",
            [id, progress],
        )?;
        Ok(())
    }

    pub fn set_tool_call_approval(&self, id: &str, approval_id: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE tool_calls SET approval_id = ?2, changed_at = unixepoch() WHERE id = ?1",
            [id, approval_id],
        )?;
        Ok(())
    }

    /// Calls left in hand longer than `secs` ago: interrupted.
    pub fn tool_calls_interrupted(&self, secs: i64) -> Result<Vec<ToolCallRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM tool_calls WHERE state = 'calling' AND changed_at < unixepoch() - ?1"
        ))?;
        let rows = statement
            .query_map([secs], read)?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_call_is_kept_through_what_becomes_of_it() {
        let db = AccountDb::in_memory().unwrap();
        db.add_tool_call(&ToolCallRow {
            id: "c1".to_owned(),
            approval_id: String::new(),
            message_id: "m1".to_owned(),
            server_id: "s1".to_owned(),
            tool: "create_ticket".to_owned(),
            call: "{}".to_owned(),
            state: "waiting".to_owned(),
            attempts: 0,
            next_try: 0,
            outcome: String::new(),
            run_id: None,
            changed_at: 0,
            progress: "[]".to_owned(),
        })
        .unwrap();
        db.set_tool_call_progress("c1", r#"[{"text":"made"}]"#)
            .unwrap();
        assert_eq!(
            db.tool_call("c1").unwrap().unwrap().progress,
            r#"[{"text":"made"}]"#
        );
        db.set_tool_call_approval("c1", "a1").unwrap();
        assert_eq!(db.tool_call_by_approval("a1").unwrap().unwrap().id, "c1");
        db.tool_call_calling("c1").unwrap();
        assert_eq!(db.tool_call("c1").unwrap().unwrap().attempts, 1);
        assert!(db.tool_calls_interrupted(60).unwrap().is_empty());
        assert_eq!(db.tool_calls_interrupted(-1).unwrap().len(), 1);
        db.set_tool_call("c1", "done", "TKT-1", 0).unwrap();
        assert_eq!(db.tool_calls_in("done").unwrap()[0].outcome, "TKT-1");
    }
}
