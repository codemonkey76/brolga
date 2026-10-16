#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Microsoft To Do through the commands (docs/spec-mail-instructions.md,
//! "Tasks"): kept here and changed offline, sent when To Do can be reached,
//! brought down by delta, and undone.

use std::sync::Arc;

use brolga_auth::{Authority, MemorySecrets};
use brolga_commands::Registry;
use brolga_core::{Core, CoreOptions};
use brolga_fake_graph::{FakeGraph, FakeIdentity, FakeMessage, Fault};
use serde_json::{Value, json};

struct Fixture {
    _identity: FakeIdentity,
    graph: FakeGraph,
    registry: Arc<Registry>,
    tasks_list: String,
}

impl Fixture {
    async fn signed_in() -> Self {
        let identity = FakeIdentity::start().await.unwrap();
        let graph = FakeGraph::start().await.unwrap();
        let core = Core::open(CoreOptions {
            data_dir: None,
            downloads_dir: None,
            secrets: Arc::new(MemorySecrets::default()),
            open_browser: Arc::new(|url| {
                let url = url.to_owned();
                tokio::spawn(async move {
                    let _ = reqwest::get(url).await;
                });
                Ok(())
            }),
            authority: Authority::new(identity.base_url()),
            graph_base: Some(graph.base_url()),
            default_client_id: Some("client".to_owned()),
            fetch_images_from_private_hosts: false,
            host_name: None,
            anthropic_base: None,
        })
        .unwrap();
        let registry = core.registry();
        registry
            .execute("accounts.sign_in", json!({}))
            .await
            .unwrap();
        let tasks_list = graph.todo_list("defaultList").unwrap();

        Self {
            _identity: identity,
            graph,
            registry,
            tasks_list,
        }
    }

    async fn run(&self, command: &str, args: Value) -> Value {
        self.registry
            .execute(command, args)
            .await
            .unwrap_or_else(|e| panic!("{command}: {e}"))
    }

    async fn refused(&self, command: &str, args: Value) -> String {
        match self.registry.execute(command, args).await {
            Ok(done) => panic!("{command} was not refused: {done}"),
            Err(e) => e.to_string(),
        }
    }

    async fn sync(&self) -> Value {
        self.run("mail.sync", json!({})).await["accounts"][0].clone()
    }

    /// Sync until the failures put in to be offline are used up.
    async fn back_online(&self) {
        for _ in 0..10 {
            if self.sync().await["ok"] == true {
                return;
            }
        }
        panic!("never back online");
    }

    async fn titles(&self, list: &str) -> Vec<String> {
        self.run(
            "tasks.list",
            json!({ "list_id": list, "include_done": true }),
        )
        .await["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["title"].as_str().unwrap().to_owned())
            .collect()
    }

    fn on_server(&self, list: &str) -> Vec<String> {
        let mut titles: Vec<String> = self
            .graph
            .todo_tasks(list)
            .iter()
            .map(|t| t["title"].as_str().unwrap().to_owned())
            .collect();
        titles.sort();
        titles
    }
}

#[tokio::test]
async fn to_do_is_brought_down_with_its_built_in_lists_and_kept_in_step() {
    let f = Fixture::signed_in().await;
    let home = f.graph.add_todo_list("Home");
    let bins = f.graph.add_todo_task(
        &home,
        json!({
            "title": "Bins out", "importance": "high",
            "dueDateTime": { "dateTime": "2026-10-20T00:00:00.0000000", "timeZone": "UTC" }
        }),
    );
    f.sync().await;

    let lists = f.run("tasks.lists.list", json!({})).await;
    let names: Vec<&str> = lists["lists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Tasks", "Flagged email", "Home"]);
    assert_eq!(lists["lists"][1]["read_only"], true);
    assert_eq!(lists["needs_sign_in"], false);
    let task = f.run("tasks.get", json!({ "task_id": bins })).await["task"].clone();
    assert_eq!(
        (task["due"].as_str(), task["importance"].as_str()),
        (Some("2026-10-20"), Some("high"))
    );

    // Changed, and then deleted, in To Do itself.
    f.graph
        .change_todo_task(&home, &bins, json!({ "title": "Bins and recycling out" }));
    f.sync().await;
    assert_eq!(f.titles(&home).await, ["Bins and recycling out"]);
    f.graph.delete_todo_task(&home, &bins);
    f.sync().await;
    assert!(f.titles(&home).await.is_empty());
}

#[tokio::test]
async fn a_task_made_offline_reaches_to_do_once_with_its_link_to_the_email() {
    let f = Fixture::signed_in().await;
    let inbox = f.graph.folder_id("inbox");
    let message = f.graph.add_message(
        &inbox,
        FakeMessage::new(
            "Your Telstra bill",
            "Telstra",
            "bills@telstra.example",
            "2026-10-15T09:00:00Z",
        ),
    );
    f.sync().await;

    // Offline: made here, and not yet there.
    f.graph.fail_next((0..20).map(|_| Fault::Unavailable));
    let made = f.run("tasks.create", json!({
        "title": "Pay Telstra", "due": "2026-10-20", "reminder_at": "2026-10-20T09:00:00+10:00", "message_id": message
    }))
    .await;
    assert_eq!(made["task"]["on_server"], false);
    assert_eq!(f.titles(&f.tasks_list).await, ["Pay Telstra"]);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Back online, a sync sends it.
    f.back_online().await;
    f.sync().await;
    let there = f.graph.todo_tasks(&f.tasks_list);
    assert_eq!(there.len(), 1);
    assert_eq!(there[0]["dueDateTime"]["dateTime"], "2026-10-20T00:00:00");
    assert_eq!(
        there[0]["reminderDateTime"]["dateTime"],
        "2026-10-19T23:00:00"
    );
    assert_eq!(there[0]["isReminderOn"], true);
    let link = &there[0]["linkedResources"][0];
    assert_eq!(
        (
            link["applicationName"].as_str(),
            link["externalId"].as_str()
        ),
        (Some("Brolga"), Some(message.as_str()))
    );
    assert!(
        link["webUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://outlook")
    );
    let here = f.run("tasks.list", json!({})).await["tasks"][0].clone();
    assert_eq!(here["on_server"], true);
    assert_eq!(here["id"], there[0]["id"]);
    assert_eq!(here["links"][0]["external_id"], message.as_str());
}

#[tokio::test]
async fn a_task_whose_making_was_not_heard_of_is_not_made_twice() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    f.graph.lose_next_task_answer();
    f.run("tasks.create", json!({ "title": "Call the plumber" }))
        .await;
    for _ in 0..3 {
        f.sync().await;
    }
    assert_eq!(f.on_server(&f.tasks_list), ["Call the plumber"]);
    assert_eq!(f.titles(&f.tasks_list).await, ["Call the plumber"]);
}

#[tokio::test]
async fn changes_are_undone_one_action_at_a_time() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let made = f
        .run("tasks.create", json!({ "title": "Book flights" }))
        .await;
    f.sync().await;
    let id = f.run("tasks.list", json!({})).await["tasks"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let changed = f
        .run(
            "tasks.update",
            json!({ "task_id": id, "title": "Book flights to Perth", "importance": "high" }),
        )
        .await;
    let done = f.run("tasks.complete", json!({ "task_id": id })).await;
    f.sync().await;
    let (_, there) = f.graph.todo_task(&id).unwrap();
    assert_eq!(
        (there["title"].as_str(), there["status"].as_str()),
        (Some("Book flights to Perth"), Some("completed"))
    );
    assert!(
        f.titles(&f.tasks_list)
            .await
            .contains(&"Book flights to Perth".to_owned())
    );
    assert!(
        f.run("tasks.list", json!({})).await["tasks"]
            .as_array()
            .unwrap()
            .is_empty(),
        "done tasks are hidden"
    );

    f.run("tasks.undo", json!({ "undo_id": done["undo_id"] }))
        .await;
    f.run("tasks.undo", json!({ "undo_id": changed["undo_id"] }))
        .await;
    f.sync().await;
    let (_, there) = f.graph.todo_task(&id).unwrap();
    assert_eq!(
        (
            there["title"].as_str(),
            there["status"].as_str(),
            there["importance"].as_str()
        ),
        (Some("Book flights"), Some("notStarted"), Some("normal"))
    );

    let deleted = f.run("tasks.delete", json!({ "task_id": id })).await;
    f.sync().await;
    assert!(f.on_server(&f.tasks_list).is_empty());
    f.run("tasks.undo", json!({ "undo_id": deleted["undo_id"] }))
        .await;
    f.sync().await;
    assert_eq!(f.on_server(&f.tasks_list), ["Book flights"]);

    // And the making itself: the task it made is gone, under its new id.
    f.run("tasks.undo", json!({ "undo_id": made["undo_id"] }))
        .await;
    f.sync().await;
    assert_eq!(
        f.on_server(&f.tasks_list),
        ["Book flights"],
        "the one made again on undo is another task"
    );
}

#[tokio::test]
async fn lists_are_made_renamed_and_deleted_but_not_the_built_in_ones() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    // Made offline, with a task in it, both waiting for the list.
    f.graph.fail_next((0..20).map(|_| Fault::Unavailable));
    let list = f
        .run("tasks.lists.create", json!({ "name": "Garden" }))
        .await["list"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.run("tasks.create", json!({ "list_id": list, "title": "Mow" }))
        .await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    f.back_online().await;
    f.sync().await;
    let garden = f.graph.todo_list("Garden").unwrap();
    assert_eq!(f.on_server(&garden), ["Mow"]);

    f.run(
        "tasks.lists.rename",
        json!({ "list_id": garden, "name": "Yard" }),
    )
    .await;
    f.sync().await;
    assert!(f.graph.todo_list("Yard").is_some());

    let error = f
        .refused(
            "tasks.lists.rename",
            json!({ "list_id": f.tasks_list, "name": "Mine" }),
        )
        .await;
    assert!(error.contains("built into To Do"), "{error}");
    let error = f
        .refused(
            "tasks.create",
            json!({ "list_id": "flaggedEmails", "title": "x" }),
        )
        .await;
    assert!(error.contains("flag"), "{error}");

    f.run("tasks.lists.delete", json!({ "list_id": garden }))
        .await;
    f.sync().await;
    assert!(f.graph.todo_list("Yard").is_none());
}

#[tokio::test]
async fn a_sign_in_from_before_tasks_is_asked_to_sign_in_again() {
    let f = Fixture::signed_in().await;
    f.graph.forbid_todo(true);
    let synced = f.sync().await;
    assert!(
        synced["tasks_error"]
            .as_str()
            .unwrap()
            .contains("Sign in to the account again")
    );
    assert_eq!(synced["ok"], true, "mail is not held up by it");
    assert_eq!(
        f.run("tasks.lists.list", json!({})).await["needs_sign_in"],
        true
    );

    f.graph.forbid_todo(false);
    f.sync().await;
    assert_eq!(
        f.run("tasks.lists.list", json!({})).await["needs_sign_in"],
        false
    );
}

#[tokio::test]
async fn a_reminder_comes_due_and_can_be_put_off_or_dismissed() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let id = f
        .run(
            "tasks.create",
            json!({ "title": "Call Mum", "reminder_at": "2026-10-20T08:00:00Z" }),
        )
        .await["task"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let due = |now: &'static str| f.run("tasks.reminders.due", json!({ "now": now }));
    assert!(
        due("2026-10-20T07:59:00Z").await["reminders"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        due("2026-10-20T08:01:00Z").await["reminders"][0]["title"],
        "Call Mum"
    );

    f.run(
        "tasks.reminders.snooze",
        json!({ "task_id": id, "until": "2026-10-20T08:30:00Z" }),
    )
    .await;
    assert!(
        due("2026-10-20T08:10:00Z").await["reminders"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        due("2026-10-20T08:31:00Z").await["reminders"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    f.run("tasks.reminders.dismiss", json!({ "task_id": id }))
        .await;
    assert!(
        due("2026-10-20T09:00:00Z").await["reminders"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_shared_mailbox_has_no_tasks_here() {
    let f = Fixture::signed_in().await;
    f.graph.add_shared_mailbox("support@example.test", true);
    let account = f.run("accounts.list", json!({})).await["accounts"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let shared = f
        .run(
            "accounts.shared.add",
            json!({ "account_id": account, "address": "support@example.test" }),
        )
        .await["mailbox"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let error = f
        .refused("tasks.lists.list", json!({ "account_id": shared }))
        .await;
    assert!(error.contains("own To Do"), "{error}");
}
