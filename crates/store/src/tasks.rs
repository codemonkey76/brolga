//! Microsoft To Do, as this computer keeps it: the lists and their tasks,
//! read and changed offline, and the changes made here queued for the
//! server, each with what undoes it (docs/spec-mail-instructions.md,
//! "Tasks"). As with mail, a change is made here and queued in one
//! transaction, and a sync does not undo a change still to be sent.

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::{AccountDb, StoreError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskListRow {
    pub id: String,
    pub name: String,
    /// `none`, `defaultList` or `flaggedEmails`.
    pub well_known: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRow {
    pub id: String,
    pub list_id: String,
    pub title: String,
    pub notes: String,
    /// `YYYY-MM-DD`, or empty.
    pub due: String,
    /// UTC, `2026-10-18T08:00:00Z`, or empty for none.
    pub reminder_at: String,
    pub importance: String,
    pub status: String,
    pub completed_at: String,
    pub created_at: String,
    pub modified_at: String,
    /// JSON.
    pub links: String,
    /// JSON.
    pub categories: String,
}

/// A queued change to To Do, as the code that sends it needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskOp {
    pub id: i64,
    pub group_id: String,
    /// `create_list`, `rename_list`, `delete_list`, `create`, `update` or `delete`.
    pub kind: String,
    pub list_id: String,
    pub task_id: String,
    pub payload: String,
    pub attempts: i64,
}

const TASK_COLUMNS: &str = "id, list_id, title, notes, due, reminder_at, importance, status, completed_at, \
     created_at, modified_at, links, categories";

fn read_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: row.get(0)?,
        list_id: row.get(1)?,
        title: row.get(2)?,
        notes: row.get(3)?,
        due: row.get(4)?,
        reminder_at: row.get(5)?,
        importance: row.get(6)?,
        status: row.get(7)?,
        completed_at: row.get(8)?,
        created_at: row.get(9)?,
        modified_at: row.get(10)?,
        links: row.get(11)?,
        categories: row.get(12)?,
    })
}

fn put(transaction: &Transaction<'_>, task: &TaskRow) -> rusqlite::Result<usize> {
    transaction.execute(
        &format!(
            "INSERT INTO tasks ({TASK_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT (id) DO UPDATE SET list_id = ?2, title = ?3, notes = ?4, due = ?5, reminder_at = ?6,
                 importance = ?7, status = ?8, completed_at = ?9, created_at = ?10, modified_at = ?11,
                 links = ?12, categories = ?13"
        ),
        params![
            task.id, task.list_id, task.title, task.notes, task.due, task.reminder_at, task.importance,
            task.status, task.completed_at, task.created_at, task.modified_at, task.links, task.categories
        ],
    )
}

fn queue(
    transaction: &Transaction<'_>,
    group: &str,
    kind: &str,
    list_id: &str,
    task_id: &str,
    payload: &str,
    before: &str,
) -> rusqlite::Result<usize> {
    transaction.execute(
        "INSERT INTO task_ops (group_id, kind, list_id, task_id, payload, before) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![group, kind, list_id, task_id, payload, before],
    )
}

/// Whether a task or list has a change here still to be sent, which a sync
/// is not to overwrite.
fn waiting(transaction: &Transaction<'_>, column: &str, id: &str) -> rusqlite::Result<bool> {
    transaction.query_row(
        &format!("SELECT EXISTS (SELECT 1 FROM task_ops WHERE state = 'queued' AND {column} = ?1)"),
        [id],
        |row| row.get(0),
    )
}

impl AccountDb {
    pub fn task_lists(&self) -> Result<Vec<TaskListRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT id, name, well_known FROM task_lists
             ORDER BY CASE well_known WHEN 'defaultList' THEN 0 WHEN 'flaggedEmails' THEN 1 ELSE 2 END, name COLLATE NOCASE",
        )?;
        let lists = statement
            .query_map([], |row| {
                Ok(TaskListRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    well_known: row.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(lists)
    }

    pub fn task_list(&self, id: &str) -> Result<Option<TaskListRow>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT id, name, well_known FROM task_lists WHERE id = ?1 OR well_known = ?1",
                [id],
                |row| {
                    Ok(TaskListRow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        well_known: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// The lists as the server has them: changed ones written, removed ones
    /// gone with their tasks; all of them at once where `whole`, when any
    /// not among them are gone too. One with a change waiting is left be.
    pub fn apply_task_lists(
        &self,
        changed: &[TaskListRow],
        removed: &[String],
        whole: bool,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        for list in changed {
            if waiting(&transaction, "list_id", &list.id)? {
                continue;
            }
            transaction.execute(
                "INSERT INTO task_lists (id, name, well_known) VALUES (?1, ?2, ?3)
                 ON CONFLICT (id) DO UPDATE SET name = ?2, well_known = ?3",
                params![list.id, list.name, list.well_known],
            )?;
        }
        let mut gone: Vec<String> = removed.to_vec();
        if whole {
            let mut statement =
                transaction.prepare("SELECT id FROM task_lists WHERE id NOT LIKE 'local-%'")?;
            let held: Vec<String> = statement
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            drop(statement);
            gone.extend(
                held.into_iter()
                    .filter(|id| !changed.iter().any(|l| &l.id == id)),
            );
        }
        for id in gone {
            if waiting(&transaction, "list_id", &id)? {
                continue;
            }
            transaction.execute("DELETE FROM tasks WHERE list_id = ?1", [&id])?;
            transaction.execute("DELETE FROM task_lists WHERE id = ?1", [&id])?;
        }
        transaction.commit()?;

        Ok(())
    }

    /// A list's tasks as the server has them, in the same way.
    pub fn apply_tasks(
        &self,
        list_id: &str,
        changed: &[TaskRow],
        removed: &[String],
        whole: bool,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        for task in changed {
            if !waiting(&transaction, "task_id", &task.id)? {
                put(&transaction, task)?;
            }
        }
        let mut gone: Vec<String> = removed.to_vec();
        if whole {
            let mut statement = transaction
                .prepare("SELECT id FROM tasks WHERE list_id = ?1 AND id NOT LIKE 'local-%'")?;
            let held: Vec<String> = statement
                .query_map([list_id], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            drop(statement);
            gone.extend(
                held.into_iter()
                    .filter(|id| !changed.iter().any(|t| &t.id == id)),
            );
        }
        for id in gone {
            if !waiting(&transaction, "task_id", &id)? {
                transaction.execute("DELETE FROM tasks WHERE id = ?1", [&id])?;
            }
        }
        transaction.commit()?;

        Ok(())
    }

    /// A list's tasks: those not done, due first and soonest, then those
    /// with no day; and the done ones after, where asked for.
    pub fn tasks(&self, list_id: &str, with_done: bool) -> Result<Vec<TaskRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM tasks WHERE list_id = ?1 AND (?2 OR status != 'completed')
             ORDER BY status = 'completed', due = '', due, created_at, id"
        ))?;
        let tasks = statement
            .query_map(params![list_id, with_done], read_task)?
            .collect::<Result<_, _>>()?;

        Ok(tasks)
    }

    pub fn task(&self, id: &str) -> Result<Option<TaskRow>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE id = ?1"),
                [id],
                read_task,
            )
            .optional()?)
    }

    /// Tasks not done with a reminder, for the reminders to come.
    pub fn tasks_with_reminders(&self) -> Result<Vec<(TaskRow, bool, Option<String>)>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {TASK_COLUMNS}, coalesce(r.dismissed, 0), r.snoozed_until FROM tasks
             LEFT JOIN task_reminders r ON r.task_id = tasks.id
             WHERE reminder_at != '' AND status != 'completed'"
        ))?;
        let rows = statement
            .query_map([], |row| Ok((read_task(row)?, row.get(13)?, row.get(14)?)))?
            .collect::<Result<_, _>>()?;

        Ok(rows)
    }

    /// A reminder dismissed, or put off until a time (UTC).
    pub fn set_task_reminder(
        &self,
        task_id: &str,
        dismissed: bool,
        snoozed_until: Option<&str>,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO task_reminders (task_id, dismissed, snoozed_until) VALUES (?1, ?2, ?3)
             ON CONFLICT (task_id) DO UPDATE SET dismissed = ?2, snoozed_until = ?3",
            params![task_id, dismissed, snoozed_until],
        )?;

        Ok(())
    }

    /// Make or change a task here, and queue what the server is to be told.
    /// `kind` is `create` or `update`; `before` is what undoes it.
    pub fn queue_task(
        &self,
        group: &str,
        kind: &str,
        task: &TaskRow,
        payload: &str,
        before: &str,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        put(&transaction, task)?;
        // A reminder set afresh is a reminder to be given again.
        transaction.execute("DELETE FROM task_reminders WHERE task_id = ?1", [&task.id])?;
        queue(
            &transaction,
            group,
            kind,
            &task.list_id,
            &task.id,
            payload,
            before,
        )?;
        transaction.commit()?;

        Ok(())
    }

    /// Delete a task here, and queue its deletion; `before` is the task as
    /// it was, to make again on undo.
    pub fn queue_task_delete(&self, group: &str, task: &TaskRow) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM tasks WHERE id = ?1", [&task.id])?;
        let before = serde_json::to_string(task).unwrap_or_default();
        queue(
            &transaction,
            group,
            "delete",
            &task.list_id,
            &task.id,
            "{}",
            &before,
        )?;
        transaction.commit()?;

        Ok(())
    }

    /// Make, rename or delete a list here, and queue it.
    pub fn queue_task_list(
        &self,
        group: &str,
        kind: &str,
        list: &TaskListRow,
        before: &str,
    ) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        match kind {
            "delete_list" => {
                transaction.execute("DELETE FROM tasks WHERE list_id = ?1", [&list.id])?;
                transaction.execute("DELETE FROM task_lists WHERE id = ?1", [&list.id])?;
            }
            _ => {
                transaction.execute(
                    "INSERT INTO task_lists (id, name, well_known) VALUES (?1, ?2, ?3)
                     ON CONFLICT (id) DO UPDATE SET name = ?2",
                    params![list.id, list.name, list.well_known],
                )?;
            }
        }
        let payload = serde_json::json!({ "displayName": list.name }).to_string();
        queue(&transaction, group, kind, &list.id, "", &payload, before)?;
        transaction.commit()?;

        Ok(())
    }

    /// The changes still to be sent, oldest first.
    pub fn queued_task_ops(&self) -> Result<Vec<TaskOp>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT id, group_id, kind, list_id, task_id, payload, attempts FROM task_ops WHERE state = 'queued' ORDER BY id",
        )?;
        let ops = statement
            .query_map([], |row| {
                Ok(TaskOp {
                    id: row.get(0)?,
                    group_id: row.get(1)?,
                    kind: row.get(2)?,
                    list_id: row.get(3)?,
                    task_id: row.get(4)?,
                    payload: row.get(5)?,
                    attempts: row.get(6)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(ops)
    }

    pub fn task_op_sent(&self, id: i64) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE task_ops SET state = 'done', error = NULL WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    /// A change that could not be sent this time, to be tried again.
    pub fn task_op_waits(&self, id: i64, error: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE task_ops SET attempts = attempts + 1, error = ?2 WHERE id = ?1",
            params![id, error],
        )?;
        Ok(())
    }

    /// A change the server will not take: given up on, and said so.
    pub fn task_op_refused(&self, id: i64, error: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE task_ops SET state = 'failed', error = ?2 WHERE id = ?1",
            params![id, error],
        )?;
        Ok(())
    }

    /// Changes the server would not take, with why: (kind, task or list, error).
    pub fn failed_task_ops(&self) -> Result<Vec<(String, String, String)>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT kind, CASE WHEN task_id = '' THEN list_id ELSE task_id END, coalesce(error, '')
             FROM task_ops WHERE state = 'failed' ORDER BY id DESC LIMIT 20",
        )?;
        let failed = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<Result<_, _>>()?;

        Ok(failed)
    }

    /// The server has given a list or task made here its own id: use it
    /// everywhere the one made here was used, queued changes included.
    pub fn replace_task_id(&self, kind: &str, local: &str, server: &str) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        if kind == "list" {
            transaction.execute(
                "UPDATE task_lists SET id = ?2 WHERE id = ?1",
                [local, server],
            )?;
            transaction.execute(
                "UPDATE tasks SET list_id = ?2 WHERE list_id = ?1",
                [local, server],
            )?;
            transaction.execute(
                "UPDATE task_ops SET list_id = ?2 WHERE list_id = ?1",
                [local, server],
            )?;
        } else {
            transaction.execute("UPDATE tasks SET id = ?2 WHERE id = ?1", [local, server])?;
            transaction.execute(
                "UPDATE task_ops SET task_id = ?2 WHERE task_id = ?1",
                [local, server],
            )?;
            transaction.execute(
                "UPDATE task_reminders SET task_id = ?2 WHERE task_id = ?1",
                [local, server],
            )?;
        }
        transaction.commit()?;

        Ok(())
    }

    /// What one action did, newest first, to undo it: (kind, list, task, before).
    pub fn task_action(
        &self,
        group: &str,
    ) -> Result<Vec<(String, String, String, String)>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT kind, list_id, task_id, before FROM task_ops WHERE group_id = ?1 AND state != 'failed' ORDER BY id DESC",
        )?;
        let ops = statement
            .query_map([group], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<_, _>>()?;

        Ok(ops)
    }

    /// Changes sent long enough ago are of no more use for undo.
    pub fn forget_old_task_ops(&self) -> Result<(), StoreError> {
        self.connection().execute(
            "DELETE FROM task_ops WHERE state != 'queued' AND created_at < unixepoch() - 86400
               AND group_id NOT IN (SELECT group_id FROM kept_undo)",
            [],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn task(id: &str, list: &str, title: &str, due: &str) -> TaskRow {
        TaskRow {
            id: id.to_owned(),
            list_id: list.to_owned(),
            title: title.to_owned(),
            due: due.to_owned(),
            importance: "normal".to_owned(),
            status: "notStarted".to_owned(),
            links: "[]".to_owned(),
            categories: "[]".to_owned(),
            ..TaskRow::default()
        }
    }

    fn list(id: &str, name: &str) -> TaskListRow {
        TaskListRow {
            id: id.to_owned(),
            name: name.to_owned(),
            well_known: "none".to_owned(),
        }
    }

    #[test]
    fn tasks_come_due_first_and_done_ones_only_when_asked() {
        let db = AccountDb::in_memory().unwrap();
        db.apply_task_lists(&[list("l", "Work")], &[], true)
            .unwrap();
        let mut done = task("d", "l", "Done", "2026-01-01");
        done.status = "completed".to_owned();
        db.apply_tasks(
            "l",
            &[
                task("a", "l", "No day", ""),
                task("b", "l", "Later", "2026-11-01"),
                task("c", "l", "Sooner", "2026-10-20"),
                done,
            ],
            &[],
            true,
        )
        .unwrap();
        let titles = |with_done| {
            db.tasks("l", with_done)
                .unwrap()
                .into_iter()
                .map(|t| t.title)
                .collect::<Vec<_>>()
        };
        assert_eq!(titles(false), ["Sooner", "Later", "No day"]);
        assert_eq!(titles(true), ["Sooner", "Later", "No day", "Done"]);
    }

    #[test]
    fn a_sync_does_not_undo_a_change_still_to_be_sent() {
        let db = AccountDb::in_memory().unwrap();
        db.apply_task_lists(&[list("l", "Work")], &[], true)
            .unwrap();
        db.apply_tasks("l", &[task("a", "l", "Old", "")], &[], true)
            .unwrap();
        db.queue_task("g", "update", &task("a", "l", "New", ""), "{}", "{}")
            .unwrap();

        // The server has not heard yet, and says what it had; and a whole
        // round that does not have it does not take it away.
        db.apply_tasks("l", &[task("a", "l", "Old", "")], &[], false)
            .unwrap();
        db.apply_tasks("l", &[], &[], true).unwrap();
        assert_eq!(db.task("a").unwrap().unwrap().title, "New");

        let op = &db.queued_task_ops().unwrap()[0];
        db.task_op_sent(op.id).unwrap();
        db.apply_tasks("l", &[], &[], true).unwrap();
        assert!(db.task("a").unwrap().is_none());
    }

    #[test]
    fn the_servers_id_replaces_the_one_made_here() {
        let db = AccountDb::in_memory().unwrap();
        db.queue_task_list("g1", "create_list", &list("local-l", "Home"), "")
            .unwrap();
        db.queue_task(
            "g2",
            "create",
            &task("local-t", "local-l", "Bins", ""),
            "{}",
            "",
        )
        .unwrap();
        db.queue_task(
            "g3",
            "update",
            &task("local-t", "local-l", "Bins out", ""),
            "{}",
            "{}",
        )
        .unwrap();

        db.replace_task_id("list", "local-l", "list-1").unwrap();
        db.replace_task_id("task", "local-t", "task-1").unwrap();
        let ops = db.queued_task_ops().unwrap();
        assert!(ops.iter().all(|op| op.list_id == "list-1"));
        assert_eq!(ops[2].task_id, "task-1");
        assert_eq!(db.task("task-1").unwrap().unwrap().title, "Bins out");
        assert_eq!(db.task_lists().unwrap()[0].id, "list-1");
    }
}
