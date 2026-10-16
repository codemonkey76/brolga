//! Microsoft To Do for the fake Graph server: task lists and their tasks,
//! and the delta queries over them, as "todoTaskList resource type",
//! "todoTask resource type" and their delta pages describe (SOURCES.md).
//! The built-in Tasks and Flagged email lists are there from the start and
//! cannot be renamed or deleted.

use std::collections::BTreeMap;

use serde_json::{Value, json};

struct List {
    name: String,
    /// `none`, `defaultList` or `flaggedEmails`.
    known: &'static str,
}

struct Change {
    seq: u64,
    id: String,
    removed: bool,
}

pub struct Todo {
    seq: u64,
    next_id: u64,
    lists: BTreeMap<String, List>,
    list_log: Vec<Change>,
    /// Each task's list, and the task as the server gives it.
    tasks: BTreeMap<String, (String, Value)>,
    /// By list.
    task_logs: BTreeMap<String, Vec<Change>>,
}

impl Default for Todo {
    fn default() -> Self {
        let mut todo = Self {
            seq: 0,
            next_id: 0,
            lists: BTreeMap::new(),
            list_log: Vec::new(),
            tasks: BTreeMap::new(),
            task_logs: BTreeMap::new(),
        };
        todo.add_list("Tasks", "defaultList");
        todo.add_list("Flagged email", "flaggedEmails");
        todo
    }
}

/// Why a change was refused, as the HTTP answer will say.
pub enum Refusal {
    NotFound,
    BuiltIn,
}

fn now() -> String {
    "2026-10-16T00:00:00Z".to_owned()
}

impl Todo {
    fn tick(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn new_id(&mut self, kind: &str) -> String {
        self.next_id += 1;
        format!("{kind}-{:04}", self.next_id)
    }

    fn log_list(&mut self, id: &str, removed: bool) {
        let seq = self.tick();
        self.list_log.push(Change {
            seq,
            id: id.to_owned(),
            removed,
        });
    }

    fn log_task(&mut self, list: &str, id: &str, removed: bool) {
        let seq = self.tick();
        self.task_logs
            .entry(list.to_owned())
            .or_default()
            .push(Change {
                seq,
                id: id.to_owned(),
                removed,
            });
    }

    pub fn add_list(&mut self, name: &str, known: &'static str) -> String {
        let id = self.new_id("list");
        self.lists.insert(
            id.clone(),
            List {
                name: name.to_owned(),
                known,
            },
        );
        self.log_list(&id, false);
        id
    }

    pub fn list_id(&self, name_or_known: &str) -> Option<String> {
        self.lists
            .iter()
            .find(|(id, l)| {
                *id == name_or_known || l.name == name_or_known || l.known == name_or_known
            })
            .map(|(id, _)| id.clone())
    }

    fn list_json(&self, id: &str) -> Option<Value> {
        let list = self.lists.get(id)?;
        Some(json!({
            "id": id, "displayName": list.name, "isOwner": true, "isShared": false,
            "wellknownListName": list.known,
            "@odata.etag": format!("W/\"{id}-{}\"", self.seq),
        }))
    }

    pub fn rename_list(&mut self, id: &str, name: &str) -> Result<Value, Refusal> {
        let list = self.lists.get_mut(id).ok_or(Refusal::NotFound)?;
        if list.known != "none" {
            return Err(Refusal::BuiltIn);
        }
        list.name = name.to_owned();
        self.log_list(id, false);
        self.list_json(id).ok_or(Refusal::NotFound)
    }

    pub fn delete_list(&mut self, id: &str) -> Result<(), Refusal> {
        let list = self.lists.get(id).ok_or(Refusal::NotFound)?;
        if list.known != "none" {
            return Err(Refusal::BuiltIn);
        }
        self.lists.remove(id);
        let gone: Vec<String> = self
            .tasks
            .iter()
            .filter(|(_, (l, _))| l == id)
            .map(|(t, _)| t.clone())
            .collect();
        for task in gone {
            self.tasks.remove(&task);
        }
        self.log_list(id, true);
        Ok(())
    }

    /// Make a task, from what a request or a test gives.
    pub fn add_task(&mut self, list: &str, given: &Value) -> Result<Value, Refusal> {
        if !self.lists.contains_key(list) {
            return Err(Refusal::NotFound);
        }
        let id = self.new_id("task");
        let mut task = json!({
            "id": id, "title": "", "body": { "content": "", "contentType": "text" },
            "importance": "normal", "status": "notStarted", "isReminderOn": false,
            "categories": [], "createdDateTime": now(), "lastModifiedDateTime": now(),
            "linkedResources": [],
        });
        merge(&mut task, given);
        if let Some(links) = task["linkedResources"].as_array_mut() {
            for (n, link) in links.iter_mut().enumerate() {
                link["id"] = json!(format!("{id}-link-{n}"));
            }
        }
        self.tasks
            .insert(id.clone(), (list.to_owned(), task.clone()));
        self.log_task(list, &id, false);
        Ok(task)
    }

    pub fn update_task(&mut self, list: &str, id: &str, changes: &Value) -> Result<Value, Refusal> {
        let (held_list, task) = self.tasks.get_mut(id).ok_or(Refusal::NotFound)?;
        if held_list != list {
            return Err(Refusal::NotFound);
        }
        merge(task, changes);
        task["lastModifiedDateTime"] = json!(now());
        if changes["status"] == "completed" && task["completedDateTime"].is_null() {
            task["completedDateTime"] =
                json!({ "dateTime": "2026-10-16T00:00:00.0000000", "timeZone": "UTC" });
        }
        let task = task.clone();
        self.log_task(list, id, false);
        Ok(task)
    }

    pub fn delete_task(&mut self, list: &str, id: &str) -> Result<(), Refusal> {
        match self.tasks.get(id) {
            Some((held, _)) if held == list => {
                self.tasks.remove(id);
                self.log_task(list, id, true);
                Ok(())
            }
            _ => Err(Refusal::NotFound),
        }
    }

    pub fn task(&self, id: &str) -> Option<(String, Value)> {
        self.tasks.get(id).cloned()
    }

    pub fn tasks_in(&self, list: &str) -> Vec<Value> {
        self.tasks
            .values()
            .filter(|(l, _)| l == list)
            .map(|(_, t)| t.clone())
            .collect()
    }

    pub fn lists(&self) -> Vec<Value> {
        self.lists
            .keys()
            .filter_map(|id| self.list_json(id))
            .collect()
    }

    /// A delta page: everything from the start, or what changed since the
    /// token, and a link for next time. One page always: test lists are short.
    pub fn lists_delta(&self, since: Option<u64>, link: &str) -> Value {
        let ids: Vec<(String, bool)> = match since {
            None => self.lists.keys().map(|id| (id.clone(), false)).collect(),
            Some(since) => latest(&self.list_log, since),
        };
        let value: Vec<Value> = ids
            .into_iter()
            .map(|(id, removed)| match self.list_json(&id) {
                Some(list) if !removed => list,
                _ => json!({ "id": id, "@removed": { "reason": "deleted" } }),
            })
            .collect();
        json!({ "value": value, "@odata.deltaLink": format!("{link}?$deltatoken={}", self.seq) })
    }

    pub fn tasks_delta(&self, list: &str, since: Option<u64>, link: &str) -> Option<Value> {
        self.lists.get(list)?;
        let ids: Vec<(String, bool)> = match since {
            None => self
                .tasks
                .iter()
                .filter(|(_, (l, _))| l == list)
                .map(|(id, _)| (id.clone(), false))
                .collect(),
            Some(since) => latest(
                self.task_logs
                    .get(list)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                since,
            ),
        };
        let value: Vec<Value> = ids
            .into_iter()
            .map(|(id, removed)| match self.tasks.get(&id) {
                Some((_, task)) if !removed => task.clone(),
                _ => json!({ "id": id, "@removed": { "reason": "deleted" } }),
            })
            .collect();
        Some(
            json!({ "value": value, "@odata.deltaLink": format!("{link}?$deltatoken={}", self.seq) }),
        )
    }
}

/// Each item once, as its latest change since `since`.
fn latest(log: &[Change], since: u64) -> Vec<(String, bool)> {
    let mut seen: Vec<(String, bool)> = Vec::new();
    for change in log.iter().filter(|c| c.seq > since) {
        seen.retain(|(id, _)| *id != change.id);
        seen.push((change.id.clone(), change.removed));
    }
    seen
}

/// The properties given, over those held: an update's way.
fn merge(held: &mut Value, given: &Value) {
    if let (Some(held), Some(given)) = (held.as_object_mut(), given.as_object()) {
        for (key, value) in given {
            if key != "id" {
                held.insert(key.clone(), value.clone());
            }
        }
    }
}
