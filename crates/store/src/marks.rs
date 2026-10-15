//! What the mail instructions have done with each message on this device,
//! and how much they have asked of a model each day
//! (docs/spec-mail-instructions.md).

use rusqlite::{OptionalExtension, params};

use crate::account::{COLUMNS, read_message};
use crate::{AccountDb, MessageRow, StoreError};

/// Where one message stands with the instructions on this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// `done`, `waiting` or `failed`.
    pub state: String,
    pub attempts: u32,
    /// When to try again, in seconds since 1970, while waiting.
    pub next_try: i64,
    pub error: String,
    pub at: i64,
}

/// How much was asked of a model on one day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub requests: u32,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

impl AccountDb {
    /// Messages in a folder that arrived at or after `since`, oldest first,
    /// that are not done or given up on, and are not waiting to be tried
    /// again later than `now`.
    pub fn arrivals_to_process(
        &self,
        folder_id: &str,
        since: &str,
        now: i64,
        limit: u32,
    ) -> Result<Vec<MessageRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {COLUMNS} FROM messages
             WHERE folder_id = ?1 AND received_at >= ?2 AND is_draft = 0
               AND NOT EXISTS (
                   SELECT 1 FROM instruction_marks k WHERE k.message_id = messages.id
                     AND (k.state != 'waiting' OR k.next_try > ?3))
             ORDER BY received_at, id LIMIT ?4"
        ))?;
        let rows = statement
            .query_map(params![folder_id, since, now, limit], read_message)?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }

    pub fn instruction_mark(&self, message_id: &str) -> Result<Option<Mark>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT state, attempts, next_try, error, at FROM instruction_marks WHERE message_id = ?1",
                [message_id],
                |row| {
                    Ok(Mark {
                        state: row.get(0)?,
                        attempts: row.get(1)?,
                        next_try: row.get(2)?,
                        error: row.get(3)?,
                        at: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// A message dealt with, by this device or another.
    pub fn mark_instruction_done(&self, message_id: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO instruction_marks (message_id, state, error) VALUES (?1, 'done', '')
             ON CONFLICT (message_id) DO UPDATE SET state = 'done', error = '', at = unixepoch()",
            [message_id],
        )?;

        Ok(())
    }

    /// A message that could not be dealt with this time: tried again from
    /// `next_try`, or given up on once `give_up` is reached. Gives back
    /// how many times it has now been tried.
    pub fn mark_instruction_failed(
        &self,
        message_id: &str,
        error: &str,
        next_try: i64,
        give_up: u32,
    ) -> Result<u32, StoreError> {
        let connection = self.connection();
        connection.execute(
            "INSERT INTO instruction_marks (message_id, state, attempts, next_try, error)
             VALUES (?1, 'waiting', 1, ?3, ?2)
             ON CONFLICT (message_id) DO UPDATE SET
                 attempts = attempts + 1, next_try = ?3, error = ?2, at = unixepoch(),
                 state = CASE WHEN attempts + 1 >= ?4 THEN 'failed' ELSE 'waiting' END",
            params![message_id, error, next_try, give_up],
        )?;

        Ok(connection.query_row(
            "SELECT attempts FROM instruction_marks WHERE message_id = ?1",
            [message_id],
            |row| row.get(0),
        )?)
    }

    /// Hold a message back until `next_try` without counting it as a
    /// failure: one over the day's cap, say.
    pub fn defer_instruction(
        &self,
        message_id: &str,
        why: &str,
        next_try: i64,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO instruction_marks (message_id, state, next_try, error) VALUES (?1, 'waiting', ?3, ?2)
             ON CONFLICT (message_id) DO UPDATE SET next_try = ?3, error = ?2, at = unixepoch()",
            params![message_id, why, next_try],
        )?;

        Ok(())
    }

    /// How many messages stand in each state: (waiting, failed).
    pub fn instruction_backlog(&self) -> Result<(u32, u32), StoreError> {
        Ok(self.connection().query_row(
            "SELECT coalesce(sum(state = 'waiting'), 0), coalesce(sum(state = 'failed'), 0) FROM instruction_marks",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }

    /// The latest trouble: the message, what went wrong and when.
    pub fn last_instruction_error(&self) -> Result<Option<(String, String, i64)>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT message_id, error, at FROM instruction_marks
                 WHERE state != 'done' AND error != '' ORDER BY at DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?)
    }

    /// Count one question asked of a model on `day` (`YYYY-MM-DD`).
    pub fn count_instruction_request(
        &self,
        day: &str,
        provider: &str,
        input_tokens: i64,
        output_tokens: i64,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO instruction_usage (day, provider, requests, input_tokens, output_tokens)
             VALUES (?1, ?2, 1, ?3, ?4)
             ON CONFLICT (day, provider) DO UPDATE SET requests = requests + 1,
                 input_tokens = input_tokens + ?3, output_tokens = output_tokens + ?4",
            params![day, provider, input_tokens, output_tokens],
        )?;

        Ok(())
    }

    pub fn instruction_usage(&self, day: &str, provider: &str) -> Result<Usage, StoreError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT requests, input_tokens, output_tokens FROM instruction_usage WHERE day = ?1 AND provider = ?2",
                [day, provider],
                |row| {
                    Ok(Usage {
                        requests: row.get(0)?,
                        input_tokens: row.get(1)?,
                        output_tokens: row.get(2)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_message_tried_too_often_is_given_up_on() {
        let db = AccountDb::in_memory().unwrap();
        assert_eq!(db.mark_instruction_failed("m1", "busy", 100, 3).unwrap(), 1);
        assert_eq!(db.instruction_mark("m1").unwrap().unwrap().state, "waiting");
        db.mark_instruction_failed("m1", "busy", 200, 3).unwrap();
        db.mark_instruction_failed("m1", "still busy", 300, 3)
            .unwrap();
        let mark = db.instruction_mark("m1").unwrap().unwrap();
        assert_eq!(
            (mark.state.as_str(), mark.attempts, mark.error.as_str()),
            ("failed", 3, "still busy")
        );
        assert_eq!(db.instruction_backlog().unwrap(), (0, 1));

        db.mark_instruction_done("m1").unwrap();
        assert_eq!(db.instruction_backlog().unwrap(), (0, 0));
    }

    #[test]
    fn requests_are_counted_by_day_and_provider() {
        let db = AccountDb::in_memory().unwrap();
        db.count_instruction_request("2026-10-16", "anthropic", 400, 40)
            .unwrap();
        db.count_instruction_request("2026-10-16", "anthropic", 100, 10)
            .unwrap();
        db.count_instruction_request("2026-10-16", "ollama", 1, 1)
            .unwrap();
        assert_eq!(
            db.instruction_usage("2026-10-16", "anthropic").unwrap(),
            Usage {
                requests: 2,
                input_tokens: 500,
                output_tokens: 50
            }
        );
        assert_eq!(
            db.instruction_usage("2026-10-17", "anthropic").unwrap(),
            Usage::default()
        );
    }
}
