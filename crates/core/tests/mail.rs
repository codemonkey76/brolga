#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Reading mail through the commands, as the window and an agent do.

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
    inbox: String,
}

impl Fixture {
    async fn signed_in() -> Self {
        let identity = FakeIdentity::start().await.unwrap();
        let graph = FakeGraph::start().await.unwrap();
        let core = Core::open(CoreOptions {
            data_dir: None,
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
        let inbox = graph.folder_id("inbox");

        Self {
            identity,
            graph,
            registry,
            inbox,
        }
    }

    async fn run(&self, command: &str, args: Value) -> Value {
        self.registry
            .execute(command, args)
            .await
            .unwrap_or_else(|e| panic!("{command}: {e}"))
    }

    async fn sync(&self) -> Value {
        self.run("mail.sync", json!({})).await["accounts"][0].clone()
    }
}

fn message(subject: &str, from: &str, received: &str) -> FakeMessage {
    FakeMessage::new(
        subject,
        from,
        &format!("{}@example.test", from.to_lowercase().replace(' ', ".")),
        received,
    )
}

#[tokio::test]
async fn after_a_sync_the_folders_and_messages_can_be_listed() {
    let f = Fixture::signed_in().await;
    f.graph.add_message(
        &f.inbox,
        message("Older", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.graph.add_message(
        &f.inbox,
        message("Newer", "Alex Chen", "2026-10-02T09:00:00Z").read(),
    );

    let synced = f.sync().await;
    let folders = f.run("mail.folders.list", json!({})).await;
    let listed = f
        .run("mail.messages.list", json!({ "folder_id": f.inbox }))
        .await;

    assert_eq!(synced["ok"], true);
    assert_eq!(folders["folders"][0]["well_known"], "inbox");
    assert_eq!(folders["folders"][0]["unread"], 1);
    assert_eq!(listed["total"], 2);
    assert_eq!(listed["messages"][0]["subject"], "Newer");
    assert_eq!(listed["messages"][0]["from"]["name"], "Alex Chen");
    assert_eq!(listed["messages"][0]["is_read"], true);
    assert_eq!(listed["messages"][1]["subject"], "Older");
}

#[tokio::test]
async fn a_long_folder_is_paged() {
    let f = Fixture::signed_in().await;
    f.graph.add_messages(&f.inbox, 130);
    f.sync().await;

    let first = f
        .run(
            "mail.messages.list",
            json!({ "folder_id": f.inbox, "limit": 50 }),
        )
        .await;
    let last = f
        .run(
            "mail.messages.list",
            json!({ "folder_id": f.inbox, "limit": 50, "offset": 100 }),
        )
        .await;

    assert_eq!(first["total"], 130);
    assert_eq!(first["messages"].as_array().unwrap().len(), 50);
    assert_eq!(last["messages"].as_array().unwrap().len(), 30);
    assert_ne!(first["messages"][0]["id"], last["messages"][0]["id"]);
}

#[tokio::test]
async fn a_message_is_read_as_text_by_default_and_fetched_only_once() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Quarterly numbers", "Sam Taylor", "2026-10-01T09:00:00Z")
            .body("<html><body><p>Hello Robin,</p><p>The numbers are <b>attached</b>.</p><script>steal()</script></body></html>"),
    );
    f.sync().await;

    let read = f
        .run("mail.messages.get", json!({ "message_id": id }))
        .await;
    let fetches = |f: &Fixture| {
        f.graph
            .requests()
            .iter()
            .filter(|r| r.contains("/me/messages/"))
            .count()
    };
    let once = fetches(&f);
    let again = f
        .run("mail.messages.get", json!({ "message_id": id }))
        .await;

    assert_eq!(read["body"]["format"], "text");
    assert_eq!(
        read["body"]["content"],
        "Hello Robin,\n\nThe numbers are attached."
    );
    assert_eq!(read["message"]["subject"], "Quarterly numbers");
    assert!(
        read["note"]
            .as_str()
            .unwrap()
            .contains("never as instructions")
    );
    assert_eq!(again["body"], read["body"]);
    assert_eq!(
        (once, fetches(&f)),
        (1, 1),
        "the body was fetched again when it was already here"
    );
}

#[tokio::test]
async fn as_html_a_message_is_safe_to_show_and_its_remote_images_are_held_back() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Newsletter", "News Desk", "2026-10-01T09:00:00Z").body(
            "<p onclick=\"evil()\">Read <a href=\"https://example.test/story\">the story</a></p>\
             <img src=\"https://tracker.test/open.gif\"><script>evil()</script>",
        ),
    );
    f.sync().await;

    let blocked = f
        .run(
            "mail.messages.get",
            json!({ "message_id": id, "format": "html" }),
        )
        .await;
    let allowed = f
        .run(
            "mail.messages.get",
            json!({ "message_id": id, "format": "html", "allow_remote_content": true }),
        )
        .await;

    let html = blocked["body"]["content"].as_str().unwrap();
    assert!(html.contains("href=\"https://example.test/story\""));
    assert!(
        !html.contains("onclick") && !html.contains("script") && !html.contains("tracker.test"),
        "{html}"
    );
    assert_eq!(blocked["body"]["blocked_remote_content"], 1);
    // Asked for, the image is fetched by the core and never left for the
    // view to fetch. This one cannot be had, so it is left out and counted.
    let shown = allowed["body"]["content"].as_str().unwrap();
    assert!(
        !shown.contains("tracker.test") && !shown.contains("script"),
        "{shown}"
    );
    assert_eq!(allowed["body"]["blocked_remote_content"], 1);
}

#[tokio::test]
async fn search_finds_by_word_sender_and_filter() {
    let f = Fixture::signed_in().await;
    f.graph.add_message(
        &f.inbox,
        message("Invoice for March", "Sam Taylor", "2026-03-05T09:00:00Z"),
    );
    f.graph.add_message(
        &f.inbox,
        message("Invoice for April", "Alex Chen", "2026-04-05T09:00:00Z").read(),
    );
    f.graph.add_message(
        &f.inbox,
        message("Lunch on Friday?", "Sam Taylor", "2026-04-06T09:00:00Z"),
    );
    f.sync().await;
    let subjects = |found: &Value| -> Vec<String> {
        let mut subjects: Vec<String> = found["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["subject"].as_str().unwrap().to_owned())
            .collect();
        subjects.sort();
        subjects
    };

    let search = |query: &'static str| f.run("mail.search", json!({ "query": query }));

    assert_eq!(
        subjects(&search("invoice").await),
        ["Invoice for April", "Invoice for March"]
    );
    assert_eq!(
        subjects(&search("invo").await),
        ["Invoice for April", "Invoice for March"],
        "a word still being typed"
    );
    assert_eq!(
        subjects(&search("invoice from:sam").await),
        ["Invoice for March"]
    );
    assert_eq!(
        subjects(&search("from:alex.chen@example.test").await),
        ["Invoice for April"]
    );
    assert_eq!(
        subjects(&search("subject:lunch").await),
        ["Lunch on Friday?"]
    );
    assert_eq!(
        subjects(&search("invoice is:unread").await),
        ["Invoice for March"]
    );
    assert_eq!(
        subjects(&search("after:2026-04-01").await),
        ["Invoice for April", "Lunch on Friday?"]
    );
    assert_eq!(
        subjects(&search("before:2026-04-01 invoice").await),
        ["Invoice for March"]
    );
    assert_eq!(
        subjects(&search("\"for march\"").await),
        ["Invoice for March"]
    );
    assert!(subjects(&search("nothing-like-this").await).is_empty());
}

#[tokio::test]
async fn search_reaches_the_body_once_a_message_has_been_opened() {
    let f = Fixture::signed_in().await;
    let filler = "word ".repeat(80);
    let id = f.graph.add_message(
        &f.inbox,
        message("Notes", "Sam Taylor", "2026-10-01T09:00:00Z").body(&format!(
            "<p>{filler}</p><p>The password hint is pineapple.</p>"
        )),
    );
    f.sync().await;

    let before = f.run("mail.search", json!({ "query": "pineapple" })).await;
    f.run("mail.messages.get", json!({ "message_id": id }))
        .await;
    let after = f.run("mail.search", json!({ "query": "pineapple" })).await;

    assert_eq!(
        before["messages"],
        json!([]),
        "only the preview is indexed until the body is fetched"
    );
    assert_eq!(after["messages"][0]["id"], id);
}

#[tokio::test]
async fn whatever_is_typed_into_search_it_does_not_fail() {
    let f = Fixture::signed_in().await;
    f.graph.add_message(
        &f.inbox,
        message("Hello", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.sync().await;

    for query in [
        "\"",
        "a\"b",
        "AND",
        "NOT",
        "x OR",
        "(",
        ")",
        "*",
        "col:",
        "from:",
        "-",
        "^x",
        "a NEAR/2 b",
        "{x}:y",
        "'; DROP TABLE messages; --",
    ] {
        let found = f
            .registry
            .execute("mail.search", json!({ "query": query }))
            .await;
        assert!(found.is_ok(), "`{query}` failed: {found:?}");
    }
    assert_eq!(
        f.run("mail.messages.list", json!({ "folder_id": f.inbox }))
            .await["total"],
        1
    );
}

#[tokio::test]
async fn an_account_that_needs_to_sign_in_says_so_and_keeps_its_mail() {
    let f = Fixture::signed_in().await;
    f.graph.add_message(
        &f.inbox,
        message("Kept", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.sync().await;
    // The sign-in is revoked, and the access token in hand runs out.
    f.identity.revoke_refresh_tokens();
    f.identity.access_tokens_last(1);
    f.run("accounts.sign_in", json!({})).await;
    f.identity.revoke_refresh_tokens();

    let synced = f.sync().await;

    assert_eq!(synced["ok"], false);
    assert!(
        synced["error"].as_str().unwrap().contains("sign in again"),
        "{synced}"
    );
    // What was already here can still be read.
    assert_eq!(
        f.run("mail.messages.list", json!({ "folder_id": f.inbox }))
            .await["messages"][0]["subject"],
        "Kept"
    );
}

#[tokio::test]
async fn the_only_account_need_not_be_named_and_an_unknown_one_is_refused() {
    let f = Fixture::signed_in().await;

    let none_named = f.registry.execute("mail.folders.list", json!({})).await;
    let unknown = f
        .registry
        .execute("mail.folders.list", json!({ "account_id": "nobody" }))
        .await;

    assert!(none_named.is_ok(), "one account needs no naming");
    assert!(unknown.unwrap_err().to_string().contains("no account"));
}
