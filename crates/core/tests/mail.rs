#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Reading mail through the commands, as the window and an agent do.

use std::sync::Arc;

use brolga_auth::{Authority, MemorySecrets};
use brolga_commands::Registry;
use brolga_core::{Core, CoreOptions};
use brolga_fake_graph::{FakeGraph, FakeIdentity, FakeMessage, Fault};
use serde_json::{Value, json};

/// A folder of this test's own to save attachments into.
fn downloads() -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    std::env::temp_dir().join(format!("brolga-mail-{}-{n}", std::process::id()))
}

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
            downloads_dir: Some(downloads()),
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
async fn folders_inside_folders_come_back_as_a_tree() {
    let f = Fixture::signed_in().await;
    let clients = f.graph.add_folder("Clients", Some(&f.inbox));
    let acme = f.graph.add_folder("Acme", Some(&clients));

    f.sync().await;
    let folders = f.run("mail.folders.list", json!({})).await;
    let folders = folders["folders"].as_array().unwrap();
    let at = |id: &str| folders.iter().position(|each| each["id"] == id).unwrap();

    assert_eq!(folders[at(&f.inbox)]["depth"], 0);
    assert_eq!(at(&clients), at(&f.inbox) + 1);
    assert_eq!(folders[at(&clients)]["depth"], 1);
    assert_eq!(at(&acme), at(&clients) + 1);
    assert_eq!(folders[at(&acme)]["depth"], 2);
}

#[tokio::test]
async fn a_folder_can_be_pinned_and_unpinned() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let pinned = |folders: Value| -> Vec<Value> {
        folders["folders"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|each| each["favourite"] == true)
            .map(|each| each["id"].clone())
            .collect()
    };

    f.run(
        "mail.folders.set_favourite",
        json!({ "folder_id": f.inbox, "favourite": true }),
    )
    .await;
    f.sync().await;
    assert_eq!(
        pinned(f.run("mail.folders.list", json!({})).await),
        [json!(f.inbox)]
    );

    f.run(
        "mail.folders.set_favourite",
        json!({ "folder_id": "inbox", "favourite": false }),
    )
    .await;
    assert!(pinned(f.run("mail.folders.list", json!({})).await).is_empty());

    let refused = f
        .registry
        .execute(
            "mail.folders.set_favourite",
            json!({ "folder_id": "no-such-folder", "favourite": true }),
        )
        .await;
    assert!(refused.is_err());
}

#[tokio::test]
async fn a_folder_can_be_read_as_conversations() {
    let f = Fixture::signed_in().await;
    let sent = f.graph.folder_id("sentitems");
    f.graph.add_message(
        &f.inbox,
        message("Lunch?", "Sam Taylor", "2026-10-01T09:00:00Z")
            .conversation("lunch")
            .read(),
    );
    f.graph.add_message(
        &sent,
        message("RE: Lunch?", "Robin Example", "2026-10-01T10:00:00Z")
            .conversation("lunch")
            .read(),
    );
    f.graph.add_message(
        &f.inbox,
        message("RE: Lunch?", "Sam Taylor", "2026-10-01T11:00:00Z").conversation("lunch"),
    );
    f.graph.add_message(
        &f.inbox,
        message("Invoice", "Alex Chen", "2026-10-02T09:00:00Z"),
    );
    f.sync().await;

    let lines = f
        .run(
            "mail.messages.list",
            json!({ "folder_id": f.inbox, "conversations": true }),
        )
        .await;
    let whole = f
        .run(
            "mail.conversations.get",
            json!({ "conversation_id": "lunch" }),
        )
        .await;
    let by_sender = f
        .run(
            "mail.messages.list",
            json!({ "folder_id": f.inbox, "sort": "sender" }),
        )
        .await;

    assert_eq!(lines["total"], 2);
    assert_eq!(lines["messages"][0]["subject"], "Invoice");
    assert_eq!(lines["messages"][1]["subject"], "RE: Lunch?");
    assert_eq!(
        lines["messages"][1]["conversation"],
        json!({ "count": 2, "unread": 1 })
    );
    let subjects: Vec<&str> = whole["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["subject"].as_str().unwrap())
        .collect();
    assert_eq!(subjects, ["Lunch?", "RE: Lunch?", "RE: Lunch?"]);
    assert_eq!(whole["messages"][1]["folder_id"], sent);
    assert_eq!(by_sender["messages"][0]["from"]["name"], "Alex Chen");
    assert_eq!(by_sender["total"], 3);
}

#[tokio::test]
async fn attachments_are_listed_without_their_contents_and_kept() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Invoice", "Alex Chen", "2026-10-02T09:00:00Z")
            .attach("invoice.pdf", "application/pdf", b"%PDF-1.7 not really")
            .attach_link("Budget.xlsx"),
    );
    f.sync().await;

    let listed = f
        .run("mail.attachments.list", json!({ "message_id": id }))
        .await;
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 50));
    let again = f
        .run("mail.attachments.list", json!({ "message_id": id }))
        .await;

    assert_eq!(listed["attachments"][0]["name"], "invoice.pdf");
    assert_eq!(listed["attachments"][0]["size"], 19);
    assert_eq!(listed["attachments"][0]["kind"], "file");
    assert_eq!(listed["attachments"][1]["kind"], "reference");
    assert!(listed["attachments"][0].get("contentBytes").is_none());
    // The second time it is answered from this computer.
    assert_eq!(again["attachments"], listed["attachments"]);
}

#[tokio::test]
async fn an_attachment_is_saved_under_a_safe_name_and_never_over_another() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Files", "Alex Chen", "2026-10-02T09:00:00Z")
            .attach(
                "../../.config/autostart/run.desktop",
                "text/plain",
                b"[Desktop Entry]",
            )
            .attach_link("Budget.xlsx"),
    );
    f.sync().await;
    let save = |attachment: &'static str| {
        f.registry.execute(
            "mail.attachments.save",
            json!({ "message_id": id, "attachment_id": attachment }),
        )
    };

    let first = save("attachment-0").await.unwrap();
    let second = save("attachment-0").await.unwrap();
    let link = save("attachment-1").await;
    let missing = save("attachment-9").await;

    let first = std::path::PathBuf::from(first["path"].as_str().unwrap());
    let second = std::path::PathBuf::from(second["path"].as_str().unwrap());
    assert_eq!(
        first.file_name().unwrap(),
        "_.._.config_autostart_run.desktop"
    );
    assert_eq!(first.parent(), second.parent());
    assert_ne!(first, second);
    assert_eq!(std::fs::read(&first).unwrap(), b"[Desktop Entry]");
    assert!(link.unwrap_err().to_string().contains("link to a file"));
    assert!(missing.is_err());

    let _ = std::fs::remove_dir_all(first.parent().unwrap());
}

#[tokio::test]
async fn with_no_window_the_core_keeps_mail_up_to_date_itself() {
    let f = Fixture::signed_in().await;
    f.graph.add_message(
        &f.inbox,
        message("First", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    let count = || async {
        f.run("mail.messages.list", json!({ "folder_id": f.inbox }))
            .await["total"]
            .as_u64()
            .unwrap()
    };
    let until = |wanted: u64| async move {
        for _ in 0..200 {
            if count().await == wanted {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        false
    };

    let syncing = tokio::spawn(brolga_core::keep_synced(
        f.registry.clone(),
        std::time::Duration::from_millis(50),
    ));

    assert!(until(1).await, "nothing was synced at the start");
    f.graph.add_message(
        &f.inbox,
        message("Second", "Sam Taylor", "2026-10-02T09:00:00Z"),
    );
    assert!(until(2).await, "new mail was not picked up later");

    syncing.abort();
}

#[tokio::test]
async fn a_picture_within_a_message_is_shown_and_kept_for_offline() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Signed", "Sam Taylor", "2026-10-01T09:00:00Z")
            .body(
                "<p>Regards</p><img src=\"cid:logo@example\"><img src=\"cid:missing@example\">\
                 <img src=\"cid:drawing@example\">",
            )
            .attach_inline("logo.png", "image/png", "logo@example", b"\x89PNG fake")
            .attach_inline("drawing.svg", "image/svg+xml", "drawing@example", b"<svg/>"),
    );
    f.sync().await;
    let read = || {
        f.run(
            "mail.messages.get",
            json!({ "message_id": id, "format": "html" }),
        )
    };

    let first = read().await;
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 50));
    let offline = read().await;
    let listed = f
        .run("mail.messages.list", json!({ "folder_id": f.inbox }))
        .await;

    let html = first["body"]["content"].as_str().unwrap();
    assert!(
        html.contains("src=\"data:image/png;base64,iVBORyBmYWtl\""),
        "{html}"
    );
    // A name with no picture, and a picture that is not a plain one, show nothing.
    assert!(!html.contains("cid:") && !html.contains("svg"), "{html}");
    assert_eq!(html.matches("<img").count(), 3);
    assert_eq!(first["body"]["blocked_remote_content"], 0);
    assert_eq!(offline["body"], first["body"]);
    assert_eq!(listed["messages"][0]["has_attachments"], false);
}

#[tokio::test]
async fn two_copies_on_one_computer_do_not_sync_the_same_account_at_once() {
    let identity = FakeIdentity::start().await.unwrap();
    let graph = FakeGraph::start().await.unwrap();
    let data_dir = downloads();
    let core = Core::open(CoreOptions {
        data_dir: Some(data_dir.clone()),
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
    let inbox = graph.folder_id("inbox");
    let sync =
        || async { registry.execute("mail.sync", json!({})).await.unwrap()["accounts"][0].clone() };
    let count = || async {
        registry
            .execute("mail.messages.list", json!({ "folder_id": inbox }))
            .await
            .unwrap()["total"]
            .clone()
    };

    assert_eq!(sync().await["syncing_elsewhere"], false);
    // Another copy is a process holding the account's lock file.
    let lock = std::fs::read_dir(data_dir.join("accounts"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|e| e == "sync-lock"))
        .expect("a lock file beside the account's database");
    let other = std::fs::File::open(&lock).unwrap();
    other.lock().unwrap();
    graph.add_message(&inbox, message("New", "Sam Taylor", "2026-10-01T09:00:00Z"));

    let left_alone = sync().await;
    assert_eq!(left_alone["ok"], true);
    assert_eq!(left_alone["syncing_elsewhere"], true);
    assert_eq!(count().await, 0);

    drop(other);
    assert_eq!(sync().await["syncing_elsewhere"], false);
    assert_eq!(count().await, 1);

    drop((registry, core));
    let _ = std::fs::remove_dir_all(&data_dir);
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
async fn a_trusted_senders_pictures_are_fetched_only_where_that_is_asked_for() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Newsletter", "News Desk", "2026-10-01T09:00:00Z")
            .body("<p>Hello</p><img src=\"https://tracker.test/open.gif\">"),
    );
    f.sync().await;
    let read = |trusting: bool| {
        let mut args = json!({ "message_id": id, "format": "html" });
        if trusting {
            args["allow_for_trusted_senders"] = json!(true);
        }
        f.run("mail.messages.get", args)
    };
    let address = read(false).await["message"]["from"]["address"]
        .as_str()
        .unwrap()
        .to_owned();

    let before = read(true).await;
    f.run(
        "mail.trusted_senders.set",
        json!({ "address": address.to_uppercase(), "trusted": true }),
    )
    .await;
    let trusting = read(true).await;
    // An agent, or anything else that does not ask, is not opted in by the list.
    let not_asking = read(false).await;
    let listed = f.run("mail.trusted_senders.list", json!({})).await;
    f.run(
        "mail.trusted_senders.set",
        json!({ "address": address, "trusted": false }),
    )
    .await;
    let after = read(true).await;
    let nonsense = f
        .registry
        .execute(
            "mail.trusted_senders.set",
            json!({ "address": "everyone", "trusted": true }),
        )
        .await;

    assert_eq!(before["remote_content_allowed"], false);
    assert_eq!(before["sender_trusted"], false);
    assert_eq!(trusting["remote_content_allowed"], true);
    assert_eq!(trusting["sender_trusted"], true);
    assert_eq!(not_asking["remote_content_allowed"], false);
    assert_eq!(listed["addresses"].as_array().unwrap().len(), 1);
    assert_eq!(after["remote_content_allowed"], false);
    assert!(nonsense.is_err());
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
async fn recent_mail_is_downloaded_in_full_so_it_reads_and_searches_offline() {
    let f = Fixture::signed_in().await;
    let filler = "word ".repeat(80);
    let recent = f.graph.add_message(
        &f.inbox,
        message("Notes", "Sam Taylor", "2026-10-01T09:00:00Z").body(&format!(
            "<p>{filler}</p><p>The password hint is pineapple.</p>"
        )),
    );
    let old = f.graph.add_message(
        &f.inbox,
        message("Old notes", "Sam Taylor", "2019-03-01T09:00:00Z")
            .body(&format!("<p>{filler}</p><p>An old hint: quince.</p>")),
    );

    let synced = f.sync().await;
    let fetches = |f: &Fixture| {
        f.graph
            .requests()
            .iter()
            .filter(|r| r.contains("/me/messages/"))
            .count()
    };
    let after_sync = fetches(&f);

    // The recent one is found by words past its preview, and opens with no request made.
    assert_eq!(synced["bodies_downloaded"], 1);
    assert_eq!(
        f.run("mail.search", json!({ "query": "pineapple" })).await["messages"][0]["id"],
        recent
    );
    f.run("mail.messages.get", json!({ "message_id": recent }))
        .await;
    assert_eq!(fetches(&f), after_sync);

    // The old one is fetched when it is opened, and searchable from then on.
    assert_eq!(
        f.run("mail.search", json!({ "query": "quince" })).await["messages"],
        json!([])
    );
    f.run("mail.messages.get", json!({ "message_id": old }))
        .await;
    assert_eq!(fetches(&f), after_sync + 1);
    assert_eq!(
        f.run("mail.search", json!({ "query": "quince" })).await["messages"][0]["id"],
        old
    );
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

#[tokio::test]
async fn sync_status_says_how_much_is_here_and_how_fresh_it_is() {
    let f = Fixture::signed_in().await;
    f.graph.add_messages(&f.inbox, 12);

    let before = f.run("app.sync.status", json!({})).await["accounts"][0].clone();
    f.sync().await;
    let after = f.run("app.sync.status", json!({})).await["accounts"][0].clone();

    assert_eq!(
        (before["messages"].clone(), before["synced_at"].clone()),
        (json!(0), Value::Null)
    );
    assert_eq!(after["username"], "robin@example.test");
    assert_eq!(
        (
            after["folders"].clone(),
            after["folders_synced"].clone(),
            after["messages"].clone()
        ),
        (json!(7), json!(7), json!(12))
    );
    assert!(after["synced_at"].as_str().unwrap().starts_with("20"));
    assert_eq!(after["syncing"], false);
    assert_eq!(after["needs_sign_in"], false);
}
