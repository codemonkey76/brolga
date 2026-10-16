#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Running the mail instructions over mail already in the mailbox
//! (docs/spec-mail-instructions.md, stage 2b): previewed, applied as
//! previewed, cancelled, undone as one, and carried on after a restart.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use brolga_auth::{Authority, MemorySecrets};
use brolga_commands::Registry;
use brolga_core::{Core, CoreOptions};
use brolga_fake_ai::{Asked, FakeAi, KEY, Reply};
use brolga_fake_graph::{FakeGraph, FakeIdentity, FakeMessage};
use serde_json::{Value, json};

const PROCESSED: &str = "String {4431b77f-55c6-4e5f-9693-e9ccc2a3f0fb} Name Processed";

async fn device(
    identity: &FakeIdentity,
    graph: &FakeGraph,
    ai: &FakeAi,
    data_dir: Option<PathBuf>,
) -> Arc<Registry> {
    let core = Core::open(CoreOptions {
        data_dir,
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
        host_name: Some("titan".to_owned()),
        anthropic_base: Some(ai.base_url()),
    })
    .unwrap();
    let registry = core.registry();
    registry
        .execute("accounts.sign_in", json!({}))
        .await
        .unwrap();
    run(
        &registry,
        "mail.instructions.model.set_key",
        json!({ "key": KEY }),
    )
    .await;

    registry
}

async fn run(on: &Registry, command: &str, args: Value) -> Value {
    on.execute_from_window(command, args)
        .await
        .unwrap_or_else(|e| panic!("{command}: {e}"))
}

async fn refused(on: &Registry, command: &str, args: Value) -> String {
    match on.execute_from_window(command, args).await {
        Ok(done) => panic!("{command} was not refused: {done}"),
        Err(e) => e.to_string(),
    }
}

async fn eventually(what: &str, mut done: impl AsyncFnMut() -> bool) {
    for _ in 0..1000 {
        if done().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what} did not happen");
}

/// Telstra's mail goes in Accounts/Telstra; nothing else is touched.
fn sensible(asked: &Asked) -> Reply {
    let ids =
        &asked.schema["properties"]["actions"]["items"]["properties"]["instruction_id"]["enum"];
    if asked.user.contains("Telstra") {
        Reply::Answer(json!({ "actions": [
            { "instruction_id": ids[0], "action": "move", "folder": "Accounts/Telstra", "categories": [], "flag_due": "", "read": false, "reason": "Telstra." }
        ]}))
    } else {
        Reply::Answer(json!({ "actions": [] }))
    }
}

struct Fixture {
    identity: FakeIdentity,
    graph: FakeGraph,
    ai: FakeAi,
    titan: Arc<Registry>,
    inbox: String,
    telstra: String,
    /// Telstra's three, then three others, all from before the instructions.
    telstras: Vec<String>,
}

impl Fixture {
    async fn set_up(data_dir: Option<PathBuf>) -> Self {
        let identity = FakeIdentity::start().await.unwrap();
        let graph = FakeGraph::start().await.unwrap();
        let ai = FakeAi::start().await.unwrap();
        ai.answer_with(sensible);
        let accounts = graph.add_folder("Accounts", None);
        let telstra = graph.add_folder("Telstra", Some(&accounts));
        let inbox = graph.folder_id("inbox");
        let mut telstras = Vec::new();
        for n in 1..=3 {
            telstras.push(graph.add_message(
                &inbox,
                FakeMessage::new(
                    &format!("Telstra bill {n}"),
                    "Telstra",
                    "bills@telstra.example",
                    &format!("2026-0{n}-05T09:00:00Z"),
                ),
            ));
        }
        for n in 1..=3 {
            graph.add_message(
                &inbox,
                FakeMessage::new(
                    &format!("Note {n}"),
                    "Sam",
                    "sam@example.test",
                    &format!("2026-0{n}-06T09:00:00Z"),
                ),
            );
        }
        let titan = device(&identity, &graph, &ai, data_dir).await;
        run(&titan, "mail.instructions.set", json!({
            "name": "Telstra", "text": "Bills from Telstra go in Accounts/Telstra.", "actions": ["move"]
        }))
        .await;
        run(&titan, "mail.instructions.processor.take", json!({})).await;
        run(
            &titan,
            "mail.instructions.model.set",
            json!({ "provider": "anthropic", "cloud_consent": true }),
        )
        .await;
        run(&titan, "mail.sync", json!({})).await;

        Self {
            identity,
            graph,
            ai,
            titan,
            inbox,
            telstra,
            telstras,
        }
    }

    fn folder_of(&self, message: &str) -> String {
        self.graph.messages()[message].0.clone()
    }

    async fn status(&self, id: &str) -> Value {
        run(
            &self.titan,
            "mail.instructions.run.status",
            json!({ "run_id": id }),
        )
        .await["run"]
            .clone()
    }

    async fn until(&self, id: &str, state: &str) -> Value {
        let mut last = Value::Null;
        eventually(&format!("the run being {state}"), async || {
            last = self.status(id).await;
            last["state"] == state
        })
        .await;
        last
    }
}

#[tokio::test]
async fn a_run_is_estimated_previewed_without_change_applied_as_shown_and_undone_as_one() {
    let f = Fixture::set_up(None).await;

    let estimate = run(
        &f.titan,
        "mail.instructions.run.estimate",
        json!({ "folder_id": "inbox" }),
    )
    .await;
    assert_eq!(estimate["messages"], 6);
    assert_eq!(estimate["not_synced"], json!([]));
    assert!(
        estimate["estimated_cost_usd"].as_f64().unwrap() < 0.05,
        "{estimate}"
    );
    let first_quarter = run(
        &f.titan,
        "mail.instructions.run.estimate",
        json!({ "folder_id": "inbox", "received_to": "2026-02-28" }),
    )
    .await;
    assert_eq!(first_quarter["messages"], 4);

    let started = run(
        &f.titan,
        "mail.instructions.run.preview",
        json!({ "folder_id": "inbox" }),
    )
    .await;
    let id = started["run"]["id"].as_str().unwrap().to_owned();
    let previewed = f.until(&id, "previewed").await;
    assert_eq!(previewed["total"], 6);
    assert_eq!(previewed["counts"], json!({ "decided": 3, "nothing": 3 }));
    assert_eq!(previewed["actions"], json!({ "move": 3 }));
    // A preview changes nothing.
    for message in &f.telstras {
        assert_eq!(f.folder_of(message), f.inbox);
        assert!(f.graph.message_property(message, PROCESSED).is_none());
    }
    assert_eq!(f.ai.asked().len(), 6);
    let changes = run(
        &f.titan,
        "mail.instructions.run.items",
        json!({ "run_id": id, "filter": "changes" }),
    )
    .await;
    assert_eq!(changes["items"].as_array().unwrap().len(), 3);
    assert_eq!(changes["items"][0]["subject"], "Telstra bill 1");
    assert_eq!(
        changes["items"][0]["decisions"][0]["folder"],
        "Accounts/Telstra"
    );

    // One is read elsewhere after its preview: it is skipped, not changed.
    f.graph.set_read(&f.telstras[2], true);
    run(&f.titan, "mail.sync", json!({})).await;

    run(
        &f.titan,
        "mail.instructions.run.apply",
        json!({ "run_id": id }),
    )
    .await;
    let applied = f.until(&id, "applied").await;
    assert_eq!(applied["counts"]["applied"], 2);
    assert_eq!(applied["counts"]["skipped"], 1);
    assert!(applied["undo_until"].is_string());
    eventually("the bills moving", async || {
        f.folder_of(&f.telstras[0]) == f.telstra && f.folder_of(&f.telstras[1]) == f.telstra
    })
    .await;
    assert_eq!(f.folder_of(&f.telstras[2]), f.inbox);
    let mark: Value =
        serde_json::from_str(&f.graph.message_property(&f.telstras[0], PROCESSED).unwrap())
            .unwrap();
    assert_eq!(mark["run_id"], id.as_str());
    let skipped = run(
        &f.titan,
        "mail.instructions.run.items",
        json!({ "run_id": id, "filter": "skipped" }),
    )
    .await;
    assert!(
        skipped["items"][0]["error"]
            .as_str()
            .unwrap()
            .contains("changed")
    );

    // Runs are not held to the day's cap for new mail, nor counted in it.
    let status = run(&f.titan, "mail.instructions.status", json!({})).await;
    assert_eq!(status["today"]["requests"], 0);

    let undone = run(
        &f.titan,
        "mail.instructions.run.undo",
        json!({ "run_id": id }),
    )
    .await;
    assert_eq!(undone["undone"], 2);
    eventually("the bills going back", async || {
        f.folder_of(&f.telstras[0]) == f.inbox && f.folder_of(&f.telstras[1]) == f.inbox
    })
    .await;
    assert_eq!(f.status(&id).await["state"], "undone");
}

#[tokio::test]
async fn a_cancelled_run_stops_and_cannot_be_applied() {
    let f = Fixture::set_up(None).await;
    // A model out of reach, so the preview is still going.
    f.ai.answer_with(|_| Reply::Status(529));
    let started = run(
        &f.titan,
        "mail.instructions.run.preview",
        json!({ "folder_id": "inbox" }),
    )
    .await;
    let id = started["run"]["id"].as_str().unwrap().to_owned();
    let other = refused(
        &f.titan,
        "mail.instructions.run.preview",
        json!({ "folder_id": "inbox" }),
    )
    .await;
    assert!(
        other.contains("already") || other.contains("previewed"),
        "{other}"
    );

    let cancelled = run(
        &f.titan,
        "mail.instructions.run.cancel",
        json!({ "run_id": id }),
    )
    .await;
    assert_eq!(cancelled["run"]["state"], "cancelled");
    let error = refused(
        &f.titan,
        "mail.instructions.run.apply",
        json!({ "run_id": id }),
    )
    .await;
    assert!(error.contains("cancelled"), "{error}");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(f.status(&id).await["state"], "cancelled");
    for message in &f.telstras {
        assert_eq!(f.folder_of(message), f.inbox);
    }
}

#[tokio::test]
async fn a_run_waits_while_the_model_is_out_of_reach_and_carries_on_after_a_restart() {
    let data = std::env::temp_dir().join(format!("brolga-runs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    let f = Fixture::set_up(Some(data.clone())).await;
    f.ai.answer_with(|_| Reply::Status(529));
    let started = run(
        &f.titan,
        "mail.instructions.run.preview",
        json!({ "folder_id": "inbox" }),
    )
    .await;
    let id = started["run"]["id"].as_str().unwrap().to_owned();
    let mut waiting = Value::Null;
    eventually("the run saying why it waits", async || {
        waiting = f.status(&id).await;
        waiting["error"].is_string()
    })
    .await;
    assert_eq!(waiting["state"], "previewing");
    assert_eq!(waiting["counts"]["pending"], 6);

    // Brolga closes; the model comes back; Brolga starts again and syncs.
    let Fixture {
        identity,
        graph,
        ai,
        titan,
        telstras,
        ..
    } = f;
    drop(titan);
    ai.answer_with(sensible);
    let again = device(&identity, &graph, &ai, Some(data.clone())).await;
    run(&again, "mail.sync", json!({})).await;
    let mut done = Value::Null;
    eventually("the run carrying on", async || {
        done = run(
            &again,
            "mail.instructions.run.status",
            json!({ "run_id": id }),
        )
        .await["run"]
            .clone();
        done["state"] == "previewed"
    })
    .await;
    assert_eq!(done["actions"], json!({ "move": 3 }));
    assert_eq!(graph.messages()[&telstras[0]].0, graph.folder_id("inbox"));
    let _ = std::fs::remove_dir_all(&data);
}

#[tokio::test]
async fn a_run_needs_the_processing_device_and_a_real_folder() {
    let f = Fixture::set_up(None).await;
    let error = refused(
        &f.titan,
        "mail.instructions.run.estimate",
        json!({ "folder_id": "no-such-folder" }),
    )
    .await;
    assert!(error.contains("no folder"), "{error}");
    let error = refused(
        &f.titan,
        "mail.instructions.run.estimate",
        json!({ "folder_id": "inbox", "received_from": "last week" }),
    )
    .await;
    assert!(error.contains("not a day"), "{error}");

    run(&f.titan, "mail.instructions.processor.release", json!({})).await;
    let error = refused(
        &f.titan,
        "mail.instructions.run.preview",
        json!({ "folder_id": "inbox" }),
    )
    .await;
    assert!(error.contains("does not carry out"), "{error}");
}
