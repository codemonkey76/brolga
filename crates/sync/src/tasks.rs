//! Keeping an account's Microsoft To Do lists and tasks in step with the
//! server, by delta ("todoTaskList: delta", "todoTask: delta", SOURCES.md).
//!
//! Each round is read whole before any of it is kept, and only its final
//! link is saved, so a round cut short is simply asked for again. The To Do
//! pages say nothing of how a deleted task is told of, so every few hours
//! the lists and tasks are read again from the beginning, and what that
//! does not bring is gone. A task or list with a change here still to be
//! sent is left as it is here.

use std::sync::Arc;

use brolga_graph::{DeltaItem, GraphClient, Next};
use brolga_store::{AccountDb, TaskListRow, TaskRow};
use jiff::Timestamp;
use serde_json::Value;

use crate::SyncError;

/// How often everything is read again from the beginning.
const WHOLE_EVERY_HOURS: i64 = 6;

/// Rounds of pages followed, at most, before giving up on one.
const MOST_PAGES: usize = 200;

pub struct TaskSync {
    graph: Arc<GraphClient>,
    db: Arc<AccountDb>,
}

/// A To Do list as it is kept here.
pub fn task_list_row(list: &Value) -> Option<TaskListRow> {
    Some(TaskListRow {
        id: list["id"].as_str()?.to_owned(),
        name: list["displayName"].as_str().unwrap_or_default().to_owned(),
        well_known: list["wellknownListName"]
            .as_str()
            .unwrap_or("none")
            .to_owned(),
    })
}

/// A moment as Graph writes one, `{dateTime, timeZone}`, in UTC.
fn moment(value: &Value) -> Option<String> {
    let at = value["dateTime"].as_str()?;
    brolga_calendar::utc(at, value["timeZone"].as_str().unwrap_or("UTC"))
}

/// A task as it is kept here.
pub fn task_row(task: &Value, list_id: &str) -> Option<TaskRow> {
    let body = &task["body"];
    let notes = match body["contentType"].as_str() {
        Some(kind) if kind.eq_ignore_ascii_case("html") => {
            brolga_mail::html_to_text(body["content"].as_str().unwrap_or_default())
        }
        _ => body["content"].as_str().unwrap_or_default().to_owned(),
    };
    // A due day is kept as Outlook and To Do keep a flag's: the day of the
    // moment in UTC (TKT-01179).
    let due = moment(&task["dueDateTime"])
        .and_then(|at| at.get(..10).map(str::to_owned))
        .unwrap_or_default();
    let reminder_at = if task["isReminderOn"] == true {
        moment(&task["reminderDateTime"]).unwrap_or_default()
    } else {
        String::new()
    };
    let links: Vec<Value> = task["linkedResources"]
        .as_array()
        .map(|all| {
            all.iter()
                .map(|link| {
                    serde_json::json!({
                        "application": link["applicationName"], "name": link["displayName"],
                        "external_id": link["externalId"], "url": link["webUrl"],
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(TaskRow {
        id: task["id"].as_str()?.to_owned(),
        list_id: list_id.to_owned(),
        title: task["title"].as_str().unwrap_or_default().to_owned(),
        notes: notes.trim_end().to_owned(),
        due,
        reminder_at,
        importance: task["importance"].as_str().unwrap_or("normal").to_owned(),
        status: task["status"].as_str().unwrap_or("notStarted").to_owned(),
        completed_at: moment(&task["completedDateTime"]).unwrap_or_default(),
        created_at: task["createdDateTime"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        modified_at: task["lastModifiedDateTime"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        links: Value::Array(links).to_string(),
        categories: task["categories"].to_string().replace("null", "[]"),
    })
}

impl TaskSync {
    pub fn new(graph: Arc<GraphClient>, db: Arc<AccountDb>) -> Self {
        Self { graph, db }
    }

    /// Bring the lists and every list's tasks up to date. Gives how many
    /// tasks changed.
    pub async fn sync_all(&self) -> Result<u32, SyncError> {
        let last_whole = self
            .db
            .setting("todo.whole_at")?
            .and_then(|at| at.parse::<Timestamp>().ok());
        let whole_due = last_whole
            .is_none_or(|at| Timestamp::now().duration_since(at).as_hours() >= WHOLE_EVERY_HOURS);
        if whole_due {
            self.db.set_setting("todo.link:lists", "")?;
            for list in self.db.task_lists()? {
                self.db.set_setting(&format!("todo.link:{}", list.id), "")?;
            }
        }

        // The lists.
        let link = self
            .db
            .setting("todo.link:lists")?
            .filter(|l| !l.is_empty());
        let whole = link.is_none();
        let (changed, removed, next) = self
            .round(link, |link| async move {
                self.graph.todo_lists_delta(link.as_deref()).await
            })
            .await?;
        let lists: Vec<TaskListRow> = changed.iter().filter_map(task_list_row).collect();
        self.db.apply_task_lists(&lists, &removed, whole)?;
        self.db.set_setting("todo.link:lists", &next)?;

        // Each list's tasks.
        let mut count = 0;
        for list in self.db.task_lists()? {
            if list.id.starts_with("local-") {
                continue;
            }
            let key = format!("todo.link:{}", list.id);
            let link = self.db.setting(&key)?.filter(|l| !l.is_empty());
            let whole = link.is_none();
            let (changed, removed, next) = self
                .round(link, |link| {
                    let id = list.id.clone();
                    async move { self.graph.todo_tasks_delta(&id, link.as_deref()).await }
                })
                .await?;
            let tasks: Vec<TaskRow> = changed
                .iter()
                .filter_map(|t| task_row(t, &list.id))
                .collect();
            count += u32::try_from(tasks.len() + removed.len()).unwrap_or(u32::MAX);
            self.db.apply_tasks(&list.id, &tasks, &removed, whole)?;
            self.db.set_setting(&key, &next)?;
        }
        if whole_due {
            self.db
                .set_setting("todo.whole_at", &Timestamp::now().to_string())?;
        }

        Ok(count)
    }

    /// One round of delta pages, whole: what changed, what went, and the
    /// link to ask next time.
    async fn round<F, Fut>(
        &self,
        link: Option<String>,
        page: F,
    ) -> Result<(Vec<Value>, Vec<String>, String), SyncError>
    where
        F: Fn(Option<String>) -> Fut,
        Fut: std::future::Future<
                Output = Result<brolga_graph::DeltaPage<Value>, brolga_graph::GraphError>,
            >,
    {
        let mut changed = Vec::new();
        let mut removed = Vec::new();
        let mut at = link;
        for _ in 0..MOST_PAGES {
            let got = page(at.clone()).await?;
            for item in got.items {
                match item {
                    DeltaItem::Changed(value) => changed.push(value),
                    DeltaItem::Removed { id } => removed.push(id),
                }
            }
            match got.next {
                Next::Page(next) => at = Some(next),
                Next::Done(next) => return Ok((changed, removed, next)),
            }
        }

        Err(SyncError::NeverSettled("To Do".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use serde_json::json;

    #[test]
    fn a_task_is_read_as_to_do_writes_it() {
        let row = task_row(
            &json!({
                "id": "t1", "title": "Pay Telstra", "importance": "high", "status": "notStarted",
                "body": { "content": "<p>Account 123</p>", "contentType": "html" },
                "dueDateTime": { "dateTime": "2026-10-20T00:00:00.0000000", "timeZone": "UTC" },
                "isReminderOn": true,
                "reminderDateTime": { "dateTime": "2026-10-19T23:00:00.0000000", "timeZone": "UTC" },
                "linkedResources": [{ "applicationName": "Brolga", "displayName": "Your bill", "externalId": "m1", "webUrl": null }]
            }),
            "l1",
        )
        .unwrap();
        assert_eq!(row.notes, "Account 123");
        assert_eq!(row.due, "2026-10-20");
        assert_eq!(row.reminder_at, "2026-10-19T23:00:00Z");
        assert_eq!(row.categories, "[]");
        let links: Value = serde_json::from_str(&row.links).unwrap();
        assert_eq!(links[0]["external_id"], "m1");

        let off = task_row(&json!({ "id": "t2", "isReminderOn": false, "reminderDateTime": { "dateTime": "2026-10-19T23:00:00", "timeZone": "UTC" } }), "l1").unwrap();
        assert_eq!(off.reminder_at, "");
    }
}
