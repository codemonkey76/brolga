#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Mail instructions as two devices signed in to one mailbox see them
//! (docs/spec-mail-instructions.md): kept in the mailbox, changed by one
//! without losing the other's change, and carried out by one device only.

use std::sync::Arc;

use brolga_auth::{Authority, MemorySecrets};
use brolga_commands::Registry;
use brolga_core::{Core, CoreOptions};
use brolga_fake_graph::{FakeGraph, FakeIdentity, FakeMessage, Fault};
use serde_json::{Value, json};

const INSTRUCTIONS: &str = "String {4431b77f-55c6-4e5f-9693-e9ccc2a3f0fb} Name Instructions";
const LAST_ACTIVE: &str = "String {4431b77f-55c6-4e5f-9693-e9ccc2a3f0fb} Name LastActive";
const PROCESSED: &str = "String {4431b77f-55c6-4e5f-9693-e9ccc2a3f0fb} Name Processed";

/// A copy of Brolga on a computer of this name, signed in to the mailbox.
async fn device(identity: &FakeIdentity, graph: &FakeGraph, name: &str) -> Arc<Registry> {
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
        host_name: Some(name.to_owned()),
    })
    .unwrap();
    let registry = core.registry();
    registry
        .execute("accounts.sign_in", json!({}))
        .await
        .unwrap();

    registry
}

struct Fixture {
    _identity: FakeIdentity,
    graph: FakeGraph,
    titan: Arc<Registry>,
    laptop: Arc<Registry>,
}

impl Fixture {
    async fn two_devices() -> Self {
        let identity = FakeIdentity::start().await.unwrap();
        let graph = FakeGraph::start().await.unwrap();
        let titan = device(&identity, &graph, "titan").await;
        let laptop = device(&identity, &graph, "laptop").await;

        Self {
            _identity: identity,
            graph,
            titan,
            laptop,
        }
    }

    /// The settings message in the hidden folder.
    fn settings(&self) -> String {
        let folder = self
            .graph
            .hidden_folder("Brolga")
            .expect("a settings folder");
        let mut messages = self.graph.messages_in(&folder);
        assert_eq!(messages.len(), 1, "one settings message");
        messages.remove(0)
    }
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

fn telstra() -> Value {
    json!({
        "name": "Telstra invoices",
        "text": "Invoices from Telstra go in Accounts/Telstra and are categorised Bills.",
        "actions": ["move", "set_categories"]
    })
}

#[tokio::test]
async fn instructions_written_on_one_device_are_read_on_another_from_a_hidden_folder() {
    let f = Fixture::two_devices().await;

    // Nothing is made in the mailbox until there is something to keep.
    let before = run(&f.laptop, "mail.instructions.list", json!({})).await;
    assert_eq!(before["set_up"], false);
    assert_eq!(before["instructions"], json!([]));
    assert!(f.graph.hidden_folder("Brolga").is_none());

    let made = run(&f.titan, "mail.instructions.set", telstra()).await;
    assert_eq!(made["version"], 1);

    let seen = run(&f.laptop, "mail.instructions.list", json!({})).await;
    assert_eq!(seen["set_up"], true);
    assert_eq!(seen["version"], 1);
    assert_eq!(seen["enabled"], true);
    assert_eq!(seen["written_by"]["device_name"], "titan");
    assert_eq!(seen["instructions"], json!([made["instruction"]]));
    assert_eq!(
        seen["instructions"][0]["actions"],
        json!(["move", "set_categories"])
    );
    assert_eq!(seen["this_device"]["name"], "laptop");

    // The folder is hidden, and not in the folder list.
    run(&f.laptop, "mail.sync", json!({})).await;
    let folders = run(&f.laptop, "mail.folders.list", json!({})).await;
    assert!(
        !folders.to_string().contains("Brolga"),
        "the settings folder is listed: {folders}"
    );
    // The settings message is a draft, and says what it is for.
    let message = f.graph.message(&f.settings()).unwrap();
    assert_eq!(message["isDraft"], true);
    assert!(
        message["body"]["content"]
            .as_str()
            .unwrap()
            .contains("Please leave it")
    );
}

#[tokio::test]
async fn a_folder_of_the_persons_own_called_brolga_is_left_alone() {
    let f = Fixture::two_devices().await;
    let own = f.graph.add_folder("Brolga", None);
    f.graph.add_message(
        &own,
        FakeMessage::new(
            "Mine",
            "Sam Taylor",
            "sam@example.test",
            "2026-10-01T09:00:00Z",
        ),
    );

    // Microsoft 365 will not have two folders of one name side by side.
    let error = refused(&f.titan, "mail.instructions.set", telstra()).await;
    assert!(
        error.contains("folder of your own called “Brolga”"),
        "{error}"
    );
    assert_eq!(f.graph.messages_in(&own).len(), 1);
    let listed = run(&f.titan, "mail.instructions.list", json!({})).await;
    assert_eq!(listed["set_up"], false);
}

#[tokio::test]
async fn instructions_are_changed_moved_turned_off_and_deleted() {
    let f = Fixture::two_devices().await;
    let first = run(&f.titan, "mail.instructions.set", telstra()).await;
    let second = run(
        &f.titan,
        "mail.instructions.set",
        json!({ "name": "Quotes", "text": "A customer asking for a quote is flagged due in two days.", "actions": ["flag"] }),
    )
    .await;
    let first_id = first["instruction"]["id"].as_str().unwrap();
    let second_id = second["instruction"]["id"].as_str().unwrap();

    let changed = run(
        &f.titan,
        "mail.instructions.set",
        json!({ "instruction_id": first_id, "name": "Telstra", "text": "Telstra invoices are marked read.", "enabled": false, "actions": ["set_read"] }),
    )
    .await;
    assert_eq!(changed["version"], 3);
    let got = run(
        &f.laptop,
        "mail.instructions.get",
        json!({ "instruction_id": first_id }),
    )
    .await;
    assert_eq!(got["instruction"]["name"], "Telstra");
    assert_eq!(got["instruction"]["enabled"], false);
    assert_eq!(got["instruction"]["actions"], json!(["set_read"]));

    let moved = run(
        &f.laptop,
        "mail.instructions.reorder",
        json!({ "instruction_id": second_id, "position": 1 }),
    )
    .await;
    let order: Vec<&str> = moved["instructions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(order, [second_id, first_id]);

    let off = run(
        &f.laptop,
        "mail.instructions.set_enabled",
        json!({ "enabled": false }),
    )
    .await;
    assert_eq!(off["enabled"], false);

    let deleted = run(
        &f.titan,
        "mail.instructions.delete",
        json!({ "instruction_id": second_id }),
    )
    .await;
    assert_eq!(deleted["deleted"], true);
    let again = run(
        &f.titan,
        "mail.instructions.delete",
        json!({ "instruction_id": second_id }),
    )
    .await;
    assert_eq!(again["deleted"], false);

    let listed = run(&f.laptop, "mail.instructions.list", json!({})).await;
    assert_eq!(listed["enabled"], false);
    assert_eq!(listed["instructions"].as_array().unwrap().len(), 1);
    assert_eq!(listed["version"], 6);
}

#[tokio::test]
async fn a_change_to_a_list_that_has_moved_on_is_refused_and_the_other_change_stays() {
    let f = Fixture::two_devices().await;
    run(&f.titan, "mail.instructions.set", telstra()).await;
    let read_on_titan = run(&f.titan, "mail.instructions.list", json!({})).await;
    let read_on_laptop = run(&f.laptop, "mail.instructions.list", json!({})).await;
    let id = read_on_titan["instructions"][0]["id"].as_str().unwrap();

    // Both have version 1 open. The laptop's change goes in first.
    run(
        &f.laptop,
        "mail.instructions.set",
        json!({ "instruction_id": id, "name": "From the laptop", "text": "Changed on the laptop.", "actions": ["flag"], "version": read_on_laptop["version"] }),
    )
    .await;
    let error = refused(
        &f.titan,
        "mail.instructions.set",
        json!({ "instruction_id": id, "name": "From titan", "text": "Changed on titan.", "actions": ["flag"], "version": read_on_titan["version"] }),
    )
    .await;
    assert!(error.contains("changed on laptop"), "{error}");

    let now = run(&f.titan, "mail.instructions.list", json!({})).await;
    assert_eq!(now["version"], 2);
    assert_eq!(now["instructions"][0]["name"], "From the laptop");
}

#[tokio::test]
async fn two_changes_written_at_the_same_moment_are_found_out_on_reading_back() {
    let f = Fixture::two_devices().await;
    run(&f.titan, "mail.instructions.set", telstra()).await;
    let settings = f.settings();

    // Titan's change is on its way when the laptop's lands over it, before
    // titan reads the list back.
    let writing = f.graph.hold("PATCH /v1.0/me/messages");
    let titan = f.titan.clone();
    let change = tokio::spawn(async move {
        titan
            .execute_from_window(
                "mail.instructions.set",
                json!({ "name": "From titan", "text": "Added on titan.", "actions": ["flag"] }),
            )
            .await
    });
    writing.reached().await;
    writing.release();
    let reading_back = f.graph.hold("GET /v1.0/me/mailFolders");
    reading_back.reached().await;
    let laptops = json!({
        "format": 1, "version": 2, "enabled": true,
        "written_by": { "device_id": "laptop-id", "device_name": "laptop" },
        "written_at": "2026-10-16T00:00:00Z",
        "instructions": []
    });
    f.graph
        .set_message_property(&settings, INSTRUCTIONS, &laptops.to_string());
    reading_back.release();

    let error = change.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("changed on laptop"), "{error}");
}

#[tokio::test]
async fn a_list_written_by_a_newer_brolga_is_neither_read_nor_written_over() {
    let f = Fixture::two_devices().await;
    run(&f.titan, "mail.instructions.set", telstra()).await;
    let settings = f.settings();
    let newer = r#"{"format":2,"version":9,"enabled":true,"instructions":[],"tools":[]}"#;
    f.graph.set_message_property(&settings, INSTRUCTIONS, newer);

    let error = refused(&f.laptop, "mail.instructions.list", json!({})).await;
    assert!(error.contains("newer version of Brolga"), "{error}");
    refused(&f.laptop, "mail.instructions.set", telstra()).await;
    assert_eq!(
        f.graph.message_property(&settings, INSTRUCTIONS).as_deref(),
        Some(newer)
    );
}

#[tokio::test]
async fn a_mailbox_holds_at_most_fifty_instructions() {
    let f = Fixture::two_devices().await;
    let settings_list = json!({
        "format": 1, "version": 1, "enabled": true, "written_by": null, "written_at": null,
        "instructions": (0..50).map(|n| json!({ "id": format!("i-{n}"), "name": format!("Number {n}"), "text": "Mark it read.", "enabled": true, "actions": ["set_read"] })).collect::<Vec<_>>()
    });
    run(&f.titan, "mail.instructions.set", telstra()).await;
    f.graph
        .set_message_property(&f.settings(), INSTRUCTIONS, &settings_list.to_string());

    let error = refused(&f.titan, "mail.instructions.set", telstra()).await;
    assert!(error.contains("at most 50"), "{error}");
}

#[tokio::test]
async fn one_device_is_chosen_by_hand_to_carry_out_the_instructions() {
    let f = Fixture::two_devices().await;

    // Nobody does until one is chosen.
    let none = run(&f.titan, "mail.instructions.list", json!({})).await;
    assert_eq!(none["processor"], Value::Null);

    // Only the person can choose, in the window.
    let outside = f
        .titan
        .execute("mail.instructions.processor.take", json!({}))
        .await
        .unwrap_err();
    assert_eq!(outside.code(), "window_only");

    let taken = run(&f.titan, "mail.instructions.processor.take", json!({})).await;
    assert_eq!(taken["processing_here"], true);
    assert_eq!(taken["processor"]["device_name"], "titan");
    assert!(taken["last_active"].is_string());

    let elsewhere = run(&f.laptop, "mail.instructions.list", json!({})).await;
    assert_eq!(elsewhere["processing_here"], false);
    assert_eq!(elsewhere["processor"]["device_name"], "titan");
    assert_eq!(
        elsewhere["processor"]["device_id"],
        taken["this_device"]["id"]
    );

    // Moving it is done by hand too, from the device that takes it.
    let moved = run(&f.laptop, "mail.instructions.processor.take", json!({})).await;
    assert_eq!(moved["processing_here"], true);
    let on_titan = run(&f.titan, "mail.instructions.list", json!({})).await;
    assert_eq!(on_titan["processing_here"], false);
    assert_eq!(on_titan["processor"]["device_name"], "laptop");

    let released = run(&f.titan, "mail.instructions.processor.release", json!({})).await;
    assert_eq!(released["processor"], Value::Null);
}

#[tokio::test]
async fn only_the_processing_device_says_when_it_was_last_active() {
    let f = Fixture::two_devices().await;
    run(&f.titan, "mail.instructions.processor.take", json!({})).await;
    let settings = f.settings();
    // Long ago, so that it is due to be said again.
    let stale = "2026-01-01T00:00:00Z";
    f.graph.set_message_property(&settings, LAST_ACTIVE, stale);

    run(&f.laptop, "mail.sync", json!({})).await;
    assert_eq!(
        f.graph.message_property(&settings, LAST_ACTIVE).as_deref(),
        Some(stale)
    );

    run(&f.titan, "mail.sync", json!({})).await;
    let said = f.graph.message_property(&settings, LAST_ACTIVE).unwrap();
    assert_ne!(said, stale);

    // Not again within the quarter hour.
    run(&f.titan, "mail.sync", json!({})).await;
    assert_eq!(
        f.graph.message_property(&settings, LAST_ACTIVE).unwrap(),
        said
    );
}

#[tokio::test]
async fn a_sync_of_a_mailbox_without_instructions_makes_nothing() {
    let f = Fixture::two_devices().await;
    run(&f.titan, "mail.sync", json!({})).await;
    run(&f.titan, "mail.sync", json!({})).await;

    assert!(f.graph.hidden_folder("Brolga").is_none());
}

#[tokio::test]
async fn out_of_reach_the_instructions_are_shown_as_last_read() {
    let f = Fixture::two_devices().await;
    run(&f.titan, "mail.instructions.set", telstra()).await;
    run(&f.laptop, "mail.sync", json!({})).await;

    f.graph.fail_next((0..40).map(|_| Fault::Unavailable));
    let shown = run(&f.laptop, "mail.instructions.list", json!({})).await;
    assert_eq!(shown["offline"], true);
    assert_eq!(shown["instructions"][0]["name"], "Telstra invoices");

    // A change needs the mailbox.
    f.graph.fail_next((0..40).map(|_| Fault::Unavailable));
    let error = refused(&f.laptop, "mail.instructions.set", telstra()).await;
    assert!(error.contains("could not be reached"), "{error}");
}

#[tokio::test]
async fn a_message_carries_the_mark_it_was_dealt_with_by() {
    let f = Fixture::two_devices().await;
    let inbox = f.graph.folder_id("inbox");
    let message = f.graph.add_message(
        &inbox,
        FakeMessage::new(
            "Invoice",
            "Telstra",
            "bills@telstra.example",
            "2026-10-15T09:00:00Z",
        ),
    );

    let unmarked = run(
        &f.titan,
        "mail.instructions.processed",
        json!({ "message_id": message }),
    )
    .await;
    assert_eq!(unmarked, json!({ "processed": false, "mark": null }));

    f.graph.set_message_property(
        &message,
        PROCESSED,
        r#"{"at":"2026-10-15T09:01:00Z","version":3,"device_id":"titan-id"}"#,
    );
    let marked = run(
        &f.laptop,
        "mail.instructions.processed",
        json!({ "message_id": message }),
    )
    .await;
    assert_eq!(marked["processed"], true);
    assert_eq!(marked["mark"]["version"], 3);
}

#[tokio::test]
async fn this_device_has_a_name_that_can_be_changed() {
    let f = Fixture::two_devices().await;
    let titan = run(&f.titan, "app.device.get", json!({})).await;
    let laptop = run(&f.laptop, "app.device.get", json!({})).await;
    assert_eq!(titan["device"]["name"], "titan");
    assert_ne!(titan["device"]["id"], laptop["device"]["id"]);

    let renamed = run(
        &f.titan,
        "app.device.rename",
        json!({ "name": "Workshop PC" }),
    )
    .await;
    assert_eq!(renamed["device"]["name"], "Workshop PC");
    assert_eq!(renamed["device"]["id"], titan["device"]["id"]);

    run(&f.titan, "mail.instructions.processor.take", json!({})).await;
    let seen = run(&f.laptop, "mail.instructions.list", json!({})).await;
    assert_eq!(seen["processor"]["device_name"], "Workshop PC");
}
