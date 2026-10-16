//! Sending the changes made to To Do on this computer, in the order they
//! were made (docs/spec-mail-instructions.md, "Tasks": offline and sync).
//!
//! A change that cannot be sent for now stops the pass, so that what was
//! made later, perhaps to a task or list this one makes, waits behind it. A
//! change the server will not take is given up on and said so, and the
//! rest go on. Making a list or a task is not safe to ask twice, so when
//! the answer to one is lost the server is asked whether it was made first.

use std::sync::Arc;

use brolga_graph::{GraphClient, GraphError};
use brolga_store::{AccountDb, StoreError, TaskOp};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::Drained;

pub struct TaskOutbox {
    graph: Arc<GraphClient>,
    db: Arc<AccountDb>,
    /// One pass at a time.
    sending: Mutex<()>,
}

enum Sent {
    /// Done. For a list or task made, the id the server gave it.
    Done(Option<String>),
    /// Not for now; the pass stops here.
    Later(String),
    /// Never: given up on.
    Refused(String),
}

impl TaskOutbox {
    pub fn new(graph: Arc<GraphClient>, db: Arc<AccountDb>) -> Self {
        Self {
            graph,
            db,
            sending: Mutex::new(()),
        }
    }

    /// Send what is queued, oldest first.
    pub async fn drain(&self) -> Result<Drained, StoreError> {
        let _turn = self.sending.lock().await;
        let mut drained = Drained::default();
        for op in self.db.queued_task_ops()? {
            match self.send(&op).await {
                Sent::Done(made) => {
                    if let Some(server) = made {
                        let kind = if op.kind == "create_list" {
                            "list"
                        } else {
                            "task"
                        };
                        let local = if kind == "list" {
                            &op.list_id
                        } else {
                            &op.task_id
                        };
                        self.db.replace_task_id(kind, local, &server)?;
                    }
                    self.db.task_op_sent(op.id)?;
                    drained.sent += 1;
                }
                Sent::Later(why) => {
                    self.db.task_op_waits(op.id, &why)?;
                    drained.waiting += 1;
                    break;
                }
                Sent::Refused(why) => {
                    self.db.task_op_refused(op.id, &why)?;
                    drained.refused += 1;
                }
            }
        }
        self.db.forget_old_task_ops()?;

        Ok(drained)
    }

    async fn send(&self, op: &TaskOp) -> Sent {
        let mut payload: Value = serde_json::from_str(&op.payload).unwrap_or(Value::Null);
        if op.kind == "create" {
            self.web_links(&mut payload).await;
        }
        // A list made here and not yet on the server: what is to go in it waits.
        if op.list_id.starts_with("local-") && op.kind != "create_list" {
            return Sent::Later("Its list has not been made on the server yet.".to_owned());
        }
        let outcome = match op.kind.as_str() {
            "create_list" => {
                let name = payload["displayName"].as_str().unwrap_or_default();
                match self.graph.create_todo_list(name).await {
                    Ok(made) => Ok(made["id"].as_str().map(str::to_owned)),
                    Err(e) if e.is_transient() => return self.found_list(name, e).await,
                    Err(e) => Err(e),
                }
            }
            "rename_list" => self
                .graph
                .rename_todo_list(
                    &op.list_id,
                    payload["displayName"].as_str().unwrap_or_default(),
                )
                .await
                .map(|()| None),
            "delete_list" => self
                .graph
                .delete_todo_list(&op.list_id)
                .await
                .map(|()| None),
            "create" => match self.graph.create_todo_task(&op.list_id, &payload).await {
                Ok(made) => Ok(made["id"].as_str().map(str::to_owned)),
                Err(e) if e.is_transient() => {
                    return self.found_task(&op.list_id, &payload, e).await;
                }
                Err(e) => Err(e),
            },
            "update" if op.task_id.starts_with("local-") => {
                return Sent::Later("The task has not been made on the server yet.".to_owned());
            }
            "update" => self
                .graph
                .update_todo_task(&op.list_id, &op.task_id, &payload)
                .await
                .map(|_| None),
            "delete" if op.task_id.starts_with("local-") => Ok(None),
            "delete" => self
                .graph
                .delete_todo_task(&op.list_id, &op.task_id)
                .await
                .map(|()| None),
            other => {
                return Sent::Refused(format!(
                    "a queued change of a kind this version does not know: {other}"
                ));
            }
        };

        match outcome {
            Ok(made) => Sent::Done(made),
            Err(e) if e.is_transient() => Sent::Later(e.to_string()),
            Err(GraphError::Refused { status: 404, .. }) => {
                Sent::Refused("It is no longer in To Do: it was deleted elsewhere.".to_owned())
            }
            Err(e) => Sent::Refused(e.to_string()),
        }
    }

    /// A task made from an email while Microsoft 365 could not be reached
    /// has no link to where the email opens on the web: it is asked for now,
    /// and the task goes without it if it cannot be had.
    async fn web_links(&self, payload: &mut Value) {
        let Some(links) = payload["linkedResources"].as_array_mut() else {
            return;
        };
        for link in links {
            if link["applicationName"] == "Brolga"
                && link["webUrl"].is_null()
                && let Some(message_id) = link["externalId"].as_str()
                && let Ok(Some(url)) = self.graph.message_web_link(message_id).await
            {
                link["webUrl"] = Value::String(url);
            }
        }
    }

    /// The answer to making a list was lost: if a list of that name is
    /// there that this computer does not know, it was made.
    async fn found_list(&self, name: &str, lost: GraphError) -> Sent {
        let Ok(page) = self.graph.todo_lists_delta(None).await else {
            return Sent::Later(lost.to_string());
        };
        let known: Vec<String> = self
            .db
            .task_lists()
            .unwrap_or_default()
            .into_iter()
            .map(|l| l.id)
            .collect();
        for item in page.items {
            if let brolga_graph::DeltaItem::Changed(list) = item
                && list["displayName"] == name
                && let Some(id) = list["id"].as_str()
                && !known.iter().any(|k| k == id)
            {
                return Sent::Done(Some(id.to_owned()));
            }
        }

        Sent::Later(lost.to_string())
    }

    /// The answer to making a task was lost: if a task of that title is in
    /// the list that this computer does not know, it was made.
    async fn found_task(&self, list_id: &str, payload: &Value, lost: GraphError) -> Sent {
        let Ok(all) = self.graph.todo_tasks(list_id).await else {
            return Sent::Later(lost.to_string());
        };
        for task in all {
            if task["title"] == payload["title"]
                && let Some(id) = task["id"].as_str()
                && self.db.task(id).ok().flatten().is_none()
            {
                return Sent::Done(Some(id.to_owned()));
            }
        }

        Sent::Later(lost.to_string())
    }
}
