#![allow(clippy::unwrap_used, clippy::expect_used)]

//! What a command will not do, whatever it is handed.

use std::sync::Arc;

use brolga_auth::{Authority, MemorySecrets};
use brolga_commands::Registry;
use brolga_core::{Core, CoreOptions};
use brolga_fake_graph::{FakeGraph, FakeIdentity, FakeMessage};
use serde_json::{Value, json};

struct Fixture {
    #[allow(dead_code)]
    identity: FakeIdentity,
    graph: FakeGraph,
    registry: Arc<Registry>,
}

impl Fixture {
    async fn signed_in(data_dir: Option<std::path::PathBuf>) -> Self {
        let identity = FakeIdentity::start().await.unwrap();
        let graph = FakeGraph::start().await.unwrap();
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
            fetch_images_from_private_hosts: true,
        })
        .unwrap();
        let registry = core.registry();
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
