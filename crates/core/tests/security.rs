#![allow(clippy::unwrap_used, clippy::expect_used)]

//! What a command will not do, whatever it is handed.

use std::sync::Arc;

use brolga_auth::{Authority, MemorySecrets};
use brolga_commands::Registry;
use brolga_core::{Core, CoreOptions};
use brolga_fake_graph::{FakeGraph, FakeIdentity, FakeMessage};

use serde_json::{Value, json};

struct Fixture {
    identity: FakeIdentity,
    graph: FakeGraph,
    registry: Arc<Registry>,
}

/// Brolga as it is when it starts, with what it kept the last time.
fn started(
    identity: &FakeIdentity,
    graph: &FakeGraph,
    data_dir: Option<std::path::PathBuf>,
    secrets: Arc<MemorySecrets>,
) -> Arc<Registry> {
    Core::open(CoreOptions {
        data_dir,
        downloads_dir: None,
        secrets,
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
        fetch_images_from_private_hosts: true,
    })
    .unwrap()
    .registry()
}

impl Fixture {
    async fn signed_in(data_dir: Option<std::path::PathBuf>) -> Self {
        let identity = FakeIdentity::start().await.unwrap();
        let graph = FakeGraph::start().await.unwrap();
        let registry = started(&identity, &graph, data_dir, Arc::default());
        registry
            .execute("accounts.sign_in", json!({}))
            .await
            .unwrap();

        Self {
            identity,
            graph,
            registry,
        }
    }

    async fn run(&self, command: &str, args: Value) -> Value {
        self.registry
            .execute(command, args)
            .await
            .unwrap_or_else(|e| panic!("{command}: {e}"))
    }
}

#[tokio::test]
async fn an_id_made_up_by_the_caller_reaches_nothing_on_the_server() {
    let f = Fixture::signed_in(None).await;
    let draft = f.graph.add_message(
        &f.graph.folder_id("drafts"),
        FakeMessage::new("Draft", "Me", "me@example.test", "2026-10-01T09:00:00Z"),
    );
    f.run("mail.sync", json!({})).await;
    let before = f.graph.requests().len();

    for hostile in [
        format!("../messages/{draft}/send#"),
        format!("../messages/{draft}/send?"),
        format!("x/../../messages/{draft}/send#"),
        format!("../messages/{draft}/permanentDelete#"),
        "..".to_owned(),
        "no-such-thing".to_owned(),
    ] {
        let dismissed = f
            .run("calendar.reminders.dismiss", json!({ "event_id": hostile }))
            .await;
        assert_eq!(dismissed["dismissed"], false, "{hostile}");
        assert_eq!(dismissed["everywhere"], false, "{hostile}");
        let snoozed = f
            .run(
                "calendar.reminders.snooze",
                json!({ "event_id": hostile, "minutes": 5 }),
            )
            .await;
        assert_eq!(snoozed["everywhere"], false, "{hostile}");
        for (command, args) in [
            ("mail.attachments.list", json!({ "message_id": hostile })),
            (
                "mail.attachments.save",
                json!({ "message_id": hostile, "attachment_id": "attachment-0" }),
            ),
        ] {
            assert!(
                f.registry.execute(command, args).await.is_err(),
                "{command} {hostile}"
            );
        }
    }

    assert!(f.graph.sent().is_empty());
    assert_eq!(f.graph.messages().len(), 1);
    assert_eq!(
        f.graph.requests()[before..],
        Vec::<String>::new(),
        "something was asked of the server"
    );
}

/// A folder of this test's own to keep Brolga's data in.
fn data_dir(name: &str) -> std::path::PathBuf {
    let folder =
        std::env::temp_dir().join(format!("brolga-security-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);

    folder
}

/// Every file in the folder mail is kept in, by name.
fn kept(data_dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(data_dir.join("accounts"))
        .map(|entries| {
            entries
                .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    names.sort();

    names
}

#[cfg(unix)]
#[tokio::test]
async fn mail_on_this_computer_is_for_its_owner_alone() {
    use std::os::unix::fs::PermissionsExt;

    let folder = data_dir("private");
    let f = Fixture::signed_in(Some(folder.clone())).await;
    f.run("mail.sync", json!({})).await;

    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&folder), 0o700);
    assert_eq!(mode(&folder.join("accounts")), 0o700);
    let names = kept(&folder);
    assert!(names.iter().any(|name| name.ends_with(".db")), "{names:?}");
    assert!(
        names.iter().any(|name| name.ends_with(".sync-lock")),
        "{names:?}"
    );
    for name in names {
        assert_eq!(mode(&folder.join("accounts").join(&name)), 0o600, "{name}");
    }

    drop(f);
    let _ = std::fs::remove_dir_all(folder);
}

fn message(subject: &str) -> FakeMessage {
    FakeMessage::new(
        subject,
        "Alex Chen",
        "alex@example.test",
        "2026-10-02T09:00:00Z",
    )
}

#[tokio::test]
async fn removing_an_account_deletes_its_mail_from_this_computer() {
    let folder = data_dir("remove");
    let f = Fixture::signed_in(Some(folder.clone())).await;
    f.graph
        .add_message(&f.graph.folder_id("inbox"), message("Mine"));
    f.graph.add_shared_mailbox("support@example.test", true);
    f.run(
        "accounts.shared.add",
        json!({ "address": "support@example.test" }),
    )
    .await;
    f.run("mail.sync", json!({})).await;
    let account = f.run("accounts.list", json!({})).await["accounts"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = kept(&folder);
    assert!(
        before.iter().filter(|name| name.ends_with(".db")).count() == 2,
        "{before:?}"
    );

    let removed = f.run("accounts.remove", json!({ "id": account })).await;

    assert_eq!(removed["removed"], true);
    assert_eq!(kept(&folder), Vec::<String>::new());
    // Nor is the shared mailbox there for whoever signs in next.
    f.run("accounts.sign_in", json!({})).await;
    let shared = f.run("accounts.shared.list", json!({})).await;
    assert_eq!(shared["mailboxes"], json!([]));

    drop(f);
    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test]
async fn a_shared_mailbox_is_removed_whatever_case_it_is_asked_for_in() {
    let folder = data_dir("case");
    let f = Fixture::signed_in(Some(folder.clone())).await;
    f.graph.add_shared_mailbox("support@example.test", true);
    let added = f
        .run(
            "accounts.shared.add",
            json!({ "address": "support@example.test" }),
        )
        .await;
    f.run("mail.sync", json!({})).await;
    let id = added["mailbox"]["id"].as_str().unwrap();
    assert!(
        kept(&folder)
            .iter()
            .any(|name| name.contains("support_example.test")),
        "{:?}",
        kept(&folder)
    );

    let removed = f
        .run(
            "accounts.shared.remove",
            json!({ "mailbox_id": id.replace("support", "SUPPORT") }),
        )
        .await;

    assert_eq!(removed["removed"], true);
    let left = kept(&folder);
    assert!(!left.iter().any(|name| name.contains("upport")), "{left:?}");
    // The account's own mail is still there.
    assert!(left.iter().any(|name| name.ends_with(".db")), "{left:?}");

    drop(f);
    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test]
async fn two_addresses_that_differ_by_a_sign_are_kept_apart() {
    let folder = data_dir("apart");
    let f = Fixture::signed_in(Some(folder.clone())).await;
    for (address, subject) in [
        ("info_sales@example.test", "Underscore"),
        ("info+sales@example.test", "Plus"),
    ] {
        f.graph.add_shared_mailbox(address, true);
        f.graph
            .in_shared(address, |mailbox| {
                let inbox = mailbox.folder_id("inbox").unwrap();
                mailbox.add_message(&inbox, message(subject));
            })
            .unwrap();
        f.run("accounts.shared.add", json!({ "address": address }))
            .await;
    }
    f.run("mail.sync", json!({})).await;

    for (address, subject) in [
        ("info_sales@example.test", "Underscore"),
        ("info+sales@example.test", "Plus"),
    ] {
        let shared = f.run("accounts.shared.list", json!({})).await;
        let id = shared["mailboxes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|each| each["address"] == address)
            .unwrap()["id"]
            .clone();
        let folders = f
            .run("mail.folders.list", json!({ "account_id": id }))
            .await;
        let inbox = folders["folders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|each| each["well_known"] == "inbox")
            .unwrap()["id"]
            .clone();
        let listed = f
            .run(
                "mail.messages.list",
                json!({ "account_id": id, "folder_id": inbox }),
            )
            .await;
        assert_eq!(listed["total"], 1, "{address}");
        assert_eq!(listed["messages"][0]["subject"], subject, "{address}");
    }
    assert_eq!(
        kept(&folder)
            .iter()
            .filter(|name| name.ends_with(".db"))
            .count(),
        3
    );

    drop(f);
    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test]
async fn mail_kept_under_a_mailbox_s_former_file_name_is_found_under_its_new_one() {
    let folder = data_dir("former");
    let identity = FakeIdentity::start().await.unwrap();
    let graph = FakeGraph::start().await.unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let address = "info+sales@example.test";
    graph.add_shared_mailbox(address, true);
    graph
        .in_shared(address, |mailbox| {
            let inbox = mailbox.folder_id("inbox").unwrap();
            mailbox.add_message(&inbox, message("Kept"));
        })
        .unwrap();
    let id = {
        let registry = started(&identity, &graph, Some(folder.clone()), secrets.clone());
        registry
            .execute("accounts.sign_in", json!({}))
            .await
            .unwrap();
        let added = registry
            .execute("accounts.shared.add", json!({ "address": address }))
            .await
            .unwrap();
        registry.execute("mail.sync", json!({})).await.unwrap();
        added["mailbox"]["id"].as_str().unwrap().to_owned()
    };
    // As an earlier version left it: under the name that `info_sales` has too.
    let accounts = folder.join("accounts");
    let new: Vec<String> = kept(&folder)
        .into_iter()
        .filter(|name| name.starts_with("__"))
        .collect();
    assert!(new.iter().any(|name| name.ends_with(".db")), "{new:?}");
    let former = id.replace(['~', '@', '+'], "_");
    for name in &new {
        let suffix = name.split_once(".db").map(|(_, after)| after);
        match suffix {
            Some(after) => std::fs::rename(
                accounts.join(name),
                accounts.join(format!("{former}.db{after}")),
            )
            .unwrap(),
            None => std::fs::remove_file(accounts.join(name)).unwrap(),
        }
    }

    // Started again, with the server out of reach: what is read is what was kept.
    let registry = started(&identity, &graph, Some(folder.clone()), secrets);
    drop(graph);
    let folders = registry
        .execute("mail.folders.list", json!({ "account_id": id }))
        .await
        .unwrap();
    let inbox = folders["folders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|each| each["well_known"] == "inbox")
        .unwrap()["id"]
        .clone();
    let listed = registry
        .execute(
            "mail.messages.list",
            json!({ "account_id": id, "folder_id": inbox }),
        )
        .await
        .unwrap();

    assert_eq!(listed["messages"][0]["subject"], "Kept");
    let now = kept(&folder);
    assert!(!now.iter().any(|name| name.starts_with(&former)), "{now:?}");

    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test]
async fn signing_in_again_as_someone_else_is_said_and_changes_nothing() {
    let f = Fixture::signed_in(None).await;
    let account = f.run("accounts.list", json!({})).await["accounts"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.identity
        .sign_in_as("00000000-0000-4000-8000-000000000002", "sam@example.test");

    let renewed = f
        .registry
        .execute("accounts.sign_in", json!({ "account_id": account }))
        .await;

    let said = renewed.unwrap_err().to_string();
    assert!(
        said.contains("sam@example.test") && said.contains("robin@example.test"),
        "{said}"
    );
    let accounts = f.run("accounts.list", json!({})).await;
    assert_eq!(accounts["accounts"].as_array().unwrap().len(), 1);
    assert_eq!(accounts["accounts"][0]["id"], account.as_str());
}
