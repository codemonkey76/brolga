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

    // A place chosen in a save dialog is used as it is.
    let chosen = first.parent().unwrap().join("chosen name.txt");
    let as_chosen = f
        .registry
        .execute(
            "mail.attachments.save",
            json!({ "message_id": id, "attachment_id": "attachment-0", "path": chosen }),
        )
        .await
        .unwrap();
    assert_eq!(as_chosen["path"], json!(chosen));
    assert_eq!(std::fs::read(&chosen).unwrap(), b"[Desktop Entry]");

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
    // Nothing is being fetched now, and every folder has been.
    assert_eq!(
        (before["now"].clone(), after["now"].clone()),
        (Value::Null, Value::Null)
    );
    let folders = f.run("mail.folders.list", json!({})).await["folders"].clone();
    assert!(
        folders
            .as_array()
            .unwrap()
            .iter()
            .all(|each| each["synced"] == true)
    );
}

#[tokio::test]
async fn marking_and_flagging_show_at_once_reach_the_server_and_can_be_undone() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Hello", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.sync().await;
    let here = || async {
        f.run("mail.messages.list", json!({ "folder_id": f.inbox }))
            .await["messages"][0]
            .clone()
    };

    // With the server out of reach, the change still shows and is kept.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 12));
    let marked = f
        .run(
            "mail.messages.set_read",
            json!({ "message_ids": [id], "is_read": true }),
        )
        .await;
    assert_eq!(marked["changed"], 1);
    assert_eq!(here().await["is_read"], true);
    assert!(!f.graph.messages()[&id].2);

    // A sync sends what is owed before it pulls, and does not flick it back.
    f.graph.fail_next([]);
    while f.run("app.outbox.status", json!({})).await["accounts"][0]["queued"] != 0 {
        f.sync().await;
    }
    assert!(f.graph.messages()[&id].2);
    assert_eq!(here().await["is_read"], true);

    f.run(
        "mail.messages.set_flag",
        json!({ "message_ids": [id], "flag": "flagged" }),
    )
    .await;
    assert_eq!(here().await["flag_status"], "flagged");

    let undone = f
        .run("mail.undo", json!({ "undo_id": marked["undo_id"] }))
        .await;
    assert_eq!(undone["undone"], 1);
    assert_eq!(here().await["is_read"], false);
    f.sync().await;
    assert!(!f.graph.messages()[&id].2);
    assert_eq!(f.graph.flag(&id).as_deref(), Some("flagged"));
}

#[tokio::test]
async fn a_change_the_server_refuses_is_taken_back_and_can_be_read_about() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Locked", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.sync().await;
    f.graph.refuse_updates_to(&id);

    f.run(
        "mail.messages.set_read",
        json!({ "message_ids": [id], "is_read": true }),
    )
    .await;
    f.sync().await;

    let status = f.run("app.outbox.status", json!({})).await;
    let refused = &status["accounts"][0]["refused"][0];
    assert_eq!(refused["subject"], "Locked");
    assert_eq!(refused["change"], "read or unread");
    let listed = f
        .run("mail.messages.list", json!({ "folder_id": f.inbox }))
        .await;
    assert_eq!(listed["messages"][0]["is_read"], false);

    f.run("app.outbox.dismiss_refused", json!({})).await;
    let after = f.run("app.outbox.status", json!({})).await;
    assert!(
        after["accounts"][0]["refused"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn deleting_is_a_move_that_shows_at_once_and_can_be_undone() {
    let f = Fixture::signed_in().await;
    let deleted = f.graph.folder_id("deleteditems");
    let id = f.graph.add_message(
        &f.inbox,
        message("Hello", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.sync().await;
    let f = &f;
    let total = |folder: String| async move {
        f.run("mail.messages.list", json!({ "folder_id": folder }))
            .await["total"]
            .clone()
    };

    let moved = f
        .run(
            "mail.messages.move",
            json!({ "message_ids": [id], "folder_id": "deleteditems" }),
        )
        .await;
    assert_eq!(moved["changed"], 1);
    assert_eq!(total(f.inbox.clone()).await, 0);
    assert_eq!(total(deleted.clone()).await, 1);

    f.sync().await;
    assert_eq!(f.graph.messages()[&id].0, deleted);
    assert_eq!(total(deleted.clone()).await, 1);

    f.run("mail.undo", json!({ "undo_id": moved["undo_id"] }))
        .await;
    assert_eq!(total(f.inbox.clone()).await, 1);
    f.sync().await;
    assert_eq!(f.graph.messages()[&id].0, f.inbox);
    assert_eq!(total(f.inbox.clone()).await, 1);
    assert_eq!(total(deleted).await, 0);

    let nowhere = f
        .registry
        .execute(
            "mail.messages.move",
            json!({ "message_ids": [id], "folder_id": "no-such-folder" }),
        )
        .await;
    assert!(nowhere.is_err());
}

#[tokio::test]
async fn a_message_is_written_sent_and_turns_up_in_sent_items() {
    let f = Fixture::signed_in().await;
    f.sync().await;

    let draft = f
        .run(
            "mail.drafts.create",
            json!({ "to": [{ "name": "Jo", "address": " jo@example.test " }], "subject": "Lunch" }),
        )
        .await["draft"]
        .clone();
    f.run(
        "mail.drafts.edit",
        json!({ "draft_id": draft["id"], "body": "Friday?\n\nRobin" }),
    )
    .await;
    assert_eq!(draft["to"][0]["address"], "jo@example.test");

    let sent = f
        .run("mail.drafts.send", json!({ "draft_id": draft["id"] }))
        .await;
    assert_eq!(sent["queued"], true);
    while f.run("app.outbox.status", json!({})).await["accounts"][0]["sending"] != 0 {
        f.sync().await;
    }

    let went = f.graph.sent();
    assert_eq!(went.len(), 1);
    assert_eq!(went[0].subject, "Lunch");
    assert_eq!(
        went[0].body_html,
        "<html><body><p>Friday?</p><p>Robin</p></body></html>"
    );
    assert!(
        f.run("mail.drafts.list", json!({})).await["drafts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    f.sync().await;
    let in_sent = f
        .run(
            "mail.messages.list",
            json!({ "folder_id": f.graph.folder_id("sentitems") }),
        )
        .await;
    assert_eq!(in_sent["messages"][0]["subject"], "Lunch");
}

#[tokio::test]
async fn an_answer_is_addressed_and_titled_and_a_message_to_nobody_is_not_sent() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Lunch", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.sync().await;
    let from = f
        .run("mail.messages.list", json!({ "folder_id": f.inbox }))
        .await["messages"][0]["from"]["address"]
        .clone();

    let reply = f
        .run(
            "mail.drafts.create",
            json!({ "kind": "reply", "message_id": id }),
        )
        .await["draft"]
        .clone();
    let forward = f
        .run(
            "mail.drafts.create",
            json!({ "kind": "forward", "message_id": id }),
        )
        .await["draft"]
        .clone();

    assert_eq!(reply["subject"], "RE: Lunch");
    assert_eq!(reply["to"][0]["address"], from);
    assert_eq!(forward["subject"], "FW: Lunch");
    assert!(forward["to"].as_array().unwrap().is_empty());

    let to_nobody = f
        .registry
        .execute("mail.drafts.send", json!({ "draft_id": forward["id"] }))
        .await;
    assert!(to_nobody.unwrap_err().to_string().contains("recipient"));
    let not_an_address = f
        .registry
        .execute(
            "mail.drafts.edit",
            json!({ "draft_id": forward["id"], "to": [{ "address": "jo at example" }] }),
        )
        .await;
    assert!(not_an_address.is_err());
    assert!(f.graph.sent().is_empty());
}

#[tokio::test]
async fn a_signature_starts_new_messages_and_answers_only_if_asked() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Lunch", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.sync().await;
    let body_of = |args: Value| async {
        f.run("mail.drafts.create", args).await["draft"]["body"]
            .as_str()
            .unwrap()
            .to_owned()
    };

    assert_eq!(body_of(json!({})).await, "");
    f.run(
        "mail.signature.set",
        json!({ "text": "Robin Example\r\nExample Pty Ltd\n\n" }),
    )
    .await;

    assert_eq!(
        body_of(json!({})).await,
        "\n\nRobin Example\nExample Pty Ltd"
    );
    assert_eq!(
        body_of(json!({ "body": "My own words." })).await,
        "My own words."
    );
    assert_eq!(
        body_of(json!({ "kind": "reply", "message_id": id })).await,
        ""
    );

    f.run(
        "mail.signature.set",
        json!({ "text": "Robin", "on_replies": true }),
    )
    .await;
    assert_eq!(
        body_of(json!({ "kind": "reply", "message_id": id })).await,
        "\n\nRobin"
    );
    assert_eq!(
        f.run("mail.signature.get", json!({})).await,
        json!({ "text": "Robin", "on_replies": true })
    );
}

#[tokio::test]
async fn a_file_is_attached_from_disk_and_goes_with_the_message() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let folder = downloads();
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("Quarterly report.pdf");
    std::fs::write(&path, b"%PDF-1.7 not really").unwrap();
    let big = folder.join("big.bin");
    std::fs::write(&big, vec![0u8; 25 * 1024 * 1024 + 1]).unwrap();

    let draft = f
        .run(
            "mail.drafts.create",
            json!({ "to": [{ "address": "jo@example.test" }], "subject": "Report" }),
        )
        .await["draft"]["id"]
        .clone();
    let added = f
        .run(
            "mail.drafts.files.add",
            json!({ "draft_id": draft, "path": path }),
        )
        .await;
    let extra = f
        .run(
            "mail.drafts.files.add",
            json!({ "draft_id": draft, "name": "../note.txt", "content_base64": "SGk=" }),
        )
        .await;
    let too_big = f
        .registry
        .execute(
            "mail.drafts.files.add",
            json!({ "draft_id": draft, "path": big }),
        )
        .await;

    assert_eq!(added["file"]["name"], "Quarterly report.pdf");
    assert_eq!(added["file"]["content_type"], "application/pdf");
    assert_eq!(added["file"]["size"], 19);
    // A name given by a caller is made a plain file name, like any other.
    assert_eq!(extra["file"]["name"], "_note.txt");
    assert!(too_big.unwrap_err().to_string().contains("over 25 MB"));

    f.run(
        "mail.drafts.files.remove",
        json!({ "draft_id": draft, "file_id": extra["file"]["id"] }),
    )
    .await;
    let kept = f.run("mail.drafts.get", json!({ "draft_id": draft })).await;
    assert_eq!(kept["draft"]["files"].as_array().unwrap().len(), 1);

    // The original can go; the draft has its own copy.
    std::fs::remove_dir_all(&folder).unwrap();
    f.run("mail.drafts.send", json!({ "draft_id": draft }))
        .await;
    while f.run("app.outbox.status", json!({})).await["accounts"][0]["sending"] != 0 {
        f.sync().await;
    }
    let sent = &f.graph.sent()[0];
    assert_eq!(sent.attachments.len(), 1);
    assert_eq!(sent.attachments[0].name, "Quarterly report.pdf");
    assert_eq!(sent.attachments[0].content, b"%PDF-1.7 not really");
}

#[tokio::test]
async fn automatic_replies_are_read_set_and_scheduled_on_the_server() {
    let f = Fixture::signed_in().await;

    let before = f.run("mail.automatic_replies.get", json!({})).await;
    assert_eq!(before["status"], "off");

    let on = f
        .run(
            "mail.automatic_replies.set",
            json!({
                "status": "scheduled",
                "internal_message": "Away until Monday.\nRobin",
                "external_message": "I am away <until> Monday.",
                "external_audience": "contacts",
                "start": "2026-10-20T08:00:00Z",
                "end": "2026-10-24T08:00:00Z"
            }),
        )
        .await;
    assert_eq!(on["status"], "scheduled");
    assert_eq!(on["internal_message"], "Away until Monday.\nRobin");
    assert_eq!(on["external_message"], "I am away <until> Monday.");
    assert_eq!(on["start"]["date_time"], "2026-10-20T08:00:00");

    let server = f.graph.automatic_replies();
    assert_eq!(server["status"], "scheduled");
    assert_eq!(server["externalAudience"], "contactsOnly");
    // What goes to the server is HTML, with nothing typed taken as markup.
    assert!(
        server["externalReplyMessage"]
            .as_str()
            .unwrap()
            .contains("&lt;until&gt;")
    );
    assert_eq!(
        server["scheduledEndDateTime"],
        json!({ "dateTime": "2026-10-24T08:00:00", "timeZone": "UTC" })
    );

    // Turning them off leaves the messages for next time.
    let off = f
        .run("mail.automatic_replies.set", json!({ "status": "off" }))
        .await;
    assert_eq!(off["status"], "off");
    assert_eq!(off["internal_message"], "Away until Monday.\nRobin");

    for bad in [
        json!({ "status": "scheduled" }),
        json!({ "status": "scheduled", "start": "2026-10-24T08:00:00Z", "end": "2026-10-20T08:00:00Z" }),
        json!({ "status": "scheduled", "start": "next week", "end": "2026-10-20T08:00:00Z" }),
    ] {
        assert!(
            f.registry
                .execute("mail.automatic_replies.set", bad)
                .await
                .is_err()
        );
    }
    assert_eq!(f.graph.automatic_replies()["status"], "disabled");
}

#[tokio::test]
async fn a_formatted_message_is_cleaned_when_kept_and_sent_with_its_formatting() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    f.run(
        "mail.signature.set",
        json!({ "text": "Robin\nExample Pty Ltd" }),
    )
    .await;

    let draft = f
        .run(
            "mail.drafts.create",
            json!({ "to": [{ "address": "jo@example.test" }], "subject": "Plan", "format": "html" }),
        )
        .await["draft"]
        .clone();
    assert_eq!(draft["format"], "html");
    // The signature of a formatted message is formatted too.
    assert_eq!(draft["body"], "<p><br></p><p>Robin<br>Example Pty Ltd</p>");

    let kept = f
        .run(
            "mail.drafts.edit",
            json!({
                "draft_id": draft["id"],
                "body": "<p>The <b>plan</b>:</p><ol><li>Go</li></ol><script>evil()</script><p onclick=\"evil()\">x</p>"
            }),
        )
        .await;
    let body = kept["draft"]["body"].as_str().unwrap();
    assert!(
        body.contains("<b>plan</b>") && body.contains("<ol><li>Go</li></ol>"),
        "{body}"
    );
    assert!(
        !body.contains("script") && !body.contains("onclick"),
        "{body}"
    );

    f.run("mail.drafts.send", json!({ "draft_id": draft["id"] }))
        .await;
    while f.run("app.outbox.status", json!({})).await["accounts"][0]["sending"] != 0 {
        f.sync().await;
    }
    let sent = &f.graph.sent()[0].body_html;
    assert!(
        sent.starts_with("<html><body><p>The <b>plan</b>:</p><ol><li>Go</li></ol>"),
        "{sent}"
    );
}

#[tokio::test]
async fn deleting_permanently_takes_a_message_from_the_server_and_from_here() {
    let f = Fixture::signed_in().await;
    let deleted = f.graph.folder_id("deleteditems");
    let id = f.graph.add_message(
        &deleted,
        message("Old", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    let kept = f.graph.add_message(
        &deleted,
        message("Keep", "Sam Taylor", "2026-10-02T09:00:00Z"),
    );
    f.sync().await;

    let gone = f
        .run(
            "mail.messages.delete_permanently",
            json!({ "message_ids": [id] }),
        )
        .await;

    assert_eq!(gone["deleted"], 1);
    assert!(!f.graph.messages().contains_key(&id));
    assert!(f.graph.messages().contains_key(&kept));
    let listed = f
        .run("mail.messages.list", json!({ "folder_id": deleted }))
        .await;
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["messages"][0]["subject"], "Keep");
    // A later sync does not bring it back, or trip over it.
    assert_eq!(f.sync().await["ok"], true);
    assert_eq!(
        f.run("mail.messages.list", json!({ "folder_id": deleted }))
            .await["total"],
        1
    );

    // With the server out of reach nothing is deleted, here or there.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 20));
    let offline = f
        .registry
        .execute(
            "mail.messages.delete_permanently",
            json!({ "message_ids": [kept] }),
        )
        .await;
    assert!(offline.is_err());
    f.graph.clear_faults();
    assert!(f.graph.messages().contains_key(&kept));
    assert_eq!(
        f.run("mail.messages.list", json!({ "folder_id": deleted }))
            .await["total"],
        1
    );
}

#[tokio::test]
async fn categories_are_listed_set_and_undone() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Quote", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.sync().await;
    let here = || async {
        f.run("mail.messages.list", json!({ "folder_id": f.inbox }))
            .await["messages"][0]["categories"]
            .clone()
    };

    let known = f.run("mail.categories.list", json!({})).await;
    assert_eq!(
        known["categories"][1],
        json!({ "name": "Clients", "color": "preset7" })
    );

    let set = f
        .run(
            "mail.messages.set_categories",
            json!({ "message_ids": [id], "categories": ["Clients", "Red category"] }),
        )
        .await;
    assert_eq!(here().await, json!(["Clients", "Red category"]));
    f.sync().await;
    assert_eq!(f.graph.categories(&id), ["Clients", "Red category"]);
    assert_eq!(here().await, json!(["Clients", "Red category"]));

    f.run("mail.undo", json!({ "undo_id": set["undo_id"] }))
        .await;
    assert_eq!(here().await, json!([]));
    f.sync().await;
    assert!(f.graph.categories(&id).is_empty());

    // Out of reach, the list is as it was when last read.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 20));
    assert_eq!(f.run("mail.categories.list", json!({})).await, known);
}

#[tokio::test]
async fn a_draft_is_kept_in_the_mailbox_while_it_is_written_and_leaves_it_when_discarded() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let drafts = f.graph.folder_id("drafts");
    let in_drafts = || {
        f.graph
            .messages()
            .values()
            .filter(|(folder, _, _)| *folder == drafts)
            .map(|(_, subject, _)| subject.clone())
            .collect::<Vec<_>>()
    };

    let draft = f
        .run(
            "mail.drafts.create",
            json!({ "to": [{ "address": "jo@example.test" }], "subject": "Plan", "body": "First thoughts." }),
        )
        .await["draft"]
        .clone();
    assert_eq!(draft["on_server"], false);

    let flushed = f.run("app.outbox.flush", json!({})).await;
    assert_eq!(flushed["drafts_saved"], 1);
    assert_eq!(in_drafts(), ["Plan"]);

    // Synced down, the copy in Drafts opens as the same draft, not a second one.
    f.sync().await;
    let listed = f
        .run("mail.messages.list", json!({ "folder_id": drafts }))
        .await;
    let opened = f
        .run(
            "mail.drafts.open",
            json!({ "message_id": listed["messages"][0]["id"] }),
        )
        .await;
    assert_eq!(opened["draft"]["id"], draft["id"]);
    assert_eq!(opened["draft"]["on_server"], true);
    assert_eq!(
        f.run("mail.drafts.list", json!({})).await["drafts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    f.run("mail.drafts.discard", json!({ "draft_id": draft["id"] }))
        .await;
    while f.run("app.outbox.status", json!({})).await["accounts"][0]["queued"] != 0 {
        f.sync().await;
    }
    assert!(in_drafts().is_empty());
    assert_eq!(
        f.graph
            .messages()
            .values()
            .filter(|(folder, _, _)| *folder == f.graph.folder_id("deleteditems"))
            .count(),
        1
    );
}

#[tokio::test]
async fn a_draft_written_in_another_mail_program_is_taken_up_changed_and_sent() {
    let f = Fixture::signed_in().await;
    let mut elsewhere = message("From Outlook", "Robin Example", "2026-10-02T09:00:00Z")
        .body("<p style=\"color:red\">Dear <b>Jo</b>,</p><script>evil()</script>");
    elsewhere.is_draft = true;
    elsewhere.to = vec![("Jo".to_owned(), "jo@example.test".to_owned())];
    elsewhere.bcc = vec![("Al".to_owned(), "al@example.test".to_owned())];
    let id = f.graph.add_message(&f.graph.folder_id("drafts"), elsewhere);
    f.sync().await;

    let draft = f.run("mail.drafts.open", json!({ "message_id": id })).await["draft"].clone();

    assert_eq!(draft["subject"], "From Outlook");
    assert_eq!(draft["to"][0]["address"], "jo@example.test");
    assert_eq!(draft["bcc"][0]["address"], "al@example.test");
    assert_eq!(draft["format"], "html");
    let body = draft["body"].as_str().unwrap();
    assert!(
        // How it looked is kept; what could run is not.
        body.contains("<b>Jo</b>") && body.contains("color:red") && !body.contains("script"),
        "{body}"
    );

    // Untouched, it is not written back over what the other program made.
    assert_eq!(
        f.run("app.outbox.flush", json!({})).await["drafts_saved"],
        0
    );

    f.run(
        "mail.drafts.edit",
        json!({ "draft_id": draft["id"], "body": "<p>Dear Jo, see you Friday.</p>" }),
    )
    .await;
    f.run("mail.drafts.send", json!({ "draft_id": draft["id"] }))
        .await;
    while f.run("app.outbox.status", json!({})).await["accounts"][0]["sending"] != 0 {
        f.sync().await;
    }
    let sent = f.graph.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].body_html.contains("see you Friday"));
    assert_eq!(
        sent[0].bcc,
        [("Al".to_owned(), "al@example.test".to_owned())]
    );
    // It was the server's own draft that went, not a copy of it.
    assert_eq!(f.graph.drafts_created(), 0);
    assert!(
        !f.graph
            .messages()
            .values()
            .any(|(folder, _, _)| *folder == f.graph.folder_id("drafts"))
    );
}

#[tokio::test]
async fn a_flag_can_be_given_a_day_changed_and_cleared_and_all_of_it_undone() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Quote", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.sync().await;
    let here = || async {
        let m = f
            .run("mail.messages.list", json!({ "folder_id": f.inbox }))
            .await["messages"][0]
            .clone();
        (
            m["flag_status"].as_str().unwrap().to_owned(),
            m["flag_due"].as_str().map(str::to_owned),
        )
    };
    let flag = |args: Value| f.run("mail.messages.set_flag", args);

    let dated = flag(json!({ "message_ids": [id], "flag": "flagged", "due": "2026-10-20" })).await;
    assert_eq!(
        here().await,
        ("flagged".to_owned(), Some("2026-10-20".to_owned()))
    );
    f.sync().await;
    assert_eq!(f.graph.flag(&id).as_deref(), Some("flagged"));
    assert_eq!(f.graph.flag_due(&id).as_deref(), Some("2026-10-20"));
    // What comes back down from the server agrees.
    assert_eq!(here().await.1.as_deref(), Some("2026-10-20"));

    flag(json!({ "message_ids": [id], "flag": "flagged", "due": "2026-11-02" })).await;
    f.sync().await;
    assert_eq!(f.graph.flag_due(&id).as_deref(), Some("2026-11-02"));

    // No day, and still flagged.
    flag(json!({ "message_ids": [id], "flag": "flagged", "due": "" })).await;
    assert_eq!(here().await, ("flagged".to_owned(), None));
    f.sync().await;
    assert_eq!(
        (f.graph.flag(&id).as_deref(), f.graph.flag_due(&id)),
        (Some("flagged"), None)
    );
    assert_eq!(here().await, ("flagged".to_owned(), None));

    // Done has no day; undoing the very first action takes the flag off altogether.
    flag(json!({ "message_ids": [id], "flag": "flagged", "due": "2026-10-20" })).await;
    flag(json!({ "message_ids": [id], "flag": "complete" })).await;
    assert_eq!(here().await, ("complete".to_owned(), None));
    let _ = dated;

    for bad in [
        json!({ "message_ids": [id], "flag": "flagged", "due": "next Tuesday" }),
        json!({ "message_ids": [id], "flag": "flagged", "due": "2026-13-01" }),
        json!({ "message_ids": [id], "flag": "complete", "due": "2026-10-20" }),
    ] {
        assert!(
            f.registry
                .execute("mail.messages.set_flag", bad)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn flagging_with_a_day_is_undone_as_one() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Quote", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.sync().await;

    let dated = f
        .run(
            "mail.messages.set_flag",
            json!({ "message_ids": [id], "flag": "flagged", "due": "2026-10-20" }),
        )
        .await;
    f.run("mail.undo", json!({ "undo_id": dated["undo_id"] }))
        .await;

    let m = f
        .run("mail.messages.list", json!({ "folder_id": f.inbox }))
        .await["messages"][0]
        .clone();
    assert_eq!(m["flag_status"], "notFlagged");
    assert_eq!(m["flag_due"], Value::Null);
    f.sync().await;
    assert_eq!(
        (f.graph.flag(&id).as_deref(), f.graph.flag_due(&id)),
        (Some("notFlagged"), None)
    );
}

#[tokio::test]
async fn a_message_is_forwarded_as_an_attachment_whole() {
    let f = Fixture::signed_in().await;
    let id = f.graph.add_message(
        &f.inbox,
        message("Notice: of / meeting", "Sam Taylor", "2026-10-01T09:00:00Z")
            .body("<p>At ten.</p>"),
    );
    f.sync().await;

    let draft = f
        .run(
            "mail.drafts.create",
            json!({ "kind": "forward_as_attachment", "message_id": id, "to": [{ "address": "jo@example.test" }] }),
        )
        .await["draft"]
        .clone();

    assert_eq!(draft["kind"], "new");
    assert_eq!(draft["subject"], "FW: Notice: of / meeting");
    // The file is named for the message, as a plain file name.
    assert_eq!(draft["files"][0]["name"], "Notice_ of _ meeting.eml");
    assert_eq!(draft["files"][0]["content_type"], "message/rfc822");

    f.run("mail.drafts.send", json!({ "draft_id": draft["id"] }))
        .await;
    while f.run("app.outbox.status", json!({})).await["accounts"][0]["sending"] != 0 {
        f.sync().await;
    }
    let sent = &f.graph.sent()[0];
    let attached = String::from_utf8(sent.attachments[0].content.clone()).unwrap();
    assert!(attached.starts_with("From: Sam Taylor"), "{attached}");
    assert!(
        attached.contains("Subject: Notice: of / meeting") && attached.ends_with("<p>At ten.</p>")
    );
    // The original is not quoted in the body as well.
    assert!(!sent.body_html.contains("At ten."));
}

#[tokio::test]
async fn a_picture_is_put_within_a_message_and_arrives_within_it() {
    use base64::Engine;
    let f = Fixture::signed_in().await;
    f.sync().await;
    let png = b"\x89PNG\r\n\x1a\nnot really a picture";
    let encoded = base64::engine::general_purpose::STANDARD.encode(png);

    let draft = f
        .run(
            "mail.drafts.create",
            json!({ "to": [{ "address": "jo@example.test" }], "subject": "Chart", "format": "html" }),
        )
        .await["draft"]["id"]
        .clone();
    let picture = f
        .run(
            "mail.drafts.files.add",
            json!({ "draft_id": draft, "name": "chart.png", "content_base64": encoded, "inline": true }),
        )
        .await["file"]
        .clone();
    let cid = picture["content_id"].as_str().unwrap();
    assert_eq!(picture["content_type"], "image/png");

    // Whatever it is called, only a real picture goes within a message.
    let not_a_picture = f
        .registry
        .execute(
            "mail.drafts.files.add",
            json!({ "draft_id": draft, "name": "evil.png", "content_base64": "PHN2Zz48L3N2Zz4=", "inline": true }),
        )
        .await;
    assert!(not_a_picture.is_err());

    let kept = f
        .run(
            "mail.drafts.edit",
            json!({ "draft_id": draft, "body": format!("<p>See the chart:</p><img src=\"cid:{cid}\">") }),
        )
        .await;
    assert!(
        kept["draft"]["body"]
            .as_str()
            .unwrap()
            .contains(&format!("src=\"cid:{cid}\""))
    );
    let back = f
        .run(
            "mail.drafts.files.get",
            json!({ "draft_id": draft, "file_id": picture["id"] }),
        )
        .await;
    assert_eq!(back["file"]["content_base64"], encoded);

    f.run("mail.drafts.send", json!({ "draft_id": draft }))
        .await;
    while f.run("app.outbox.status", json!({})).await["accounts"][0]["sending"] != 0 {
        f.sync().await;
    }
    let sent = &f.graph.sent()[0];
    assert!(
        sent.body_html.contains(&format!("<img src=\"cid:{cid}\">")),
        "{}",
        sent.body_html
    );
    assert_eq!(sent.attachments.len(), 1);
    assert!(sent.attachments[0].inline);
    assert_eq!(sent.attachments[0].content_id.as_deref(), Some(cid));
    assert_eq!(sent.attachments[0].content, png);
    assert!(
        !sent.has_attachments,
        "a picture in the body is not an attachment beside it"
    );
}

#[tokio::test]
async fn the_calendar_syncs_with_the_mail_and_is_read_by_the_stretch_of_time() {
    use brolga_fake_graph::FakeEvent;
    let f = Fixture::signed_in().await;
    let main = f.graph.default_calendar();
    let work = f.graph.add_calendar("Work", "#2f6fce");
    // In a fortnight, whenever this is run, so it is inside what is kept.
    let day = |days: i64, hour: i8| {
        let at = jiff::Timestamp::now()
            .checked_add(jiff::Span::new().hours(24 * days))
            .unwrap()
            .strftime("%Y-%m-%d")
            .to_string();
        format!("{at}T{hour:02}:00:00Z")
    };
    let dentist = f.graph.add_event(
        &main,
        FakeEvent::new("Dentist", &day(14, 1), &day(14, 2)).at("Harbour Dental"),
    );
    f.graph
        .add_event(&work, FakeEvent::new("Shift", &day(14, 3), &day(14, 9)));
    f.graph
        .add_event(&main, FakeEvent::new("Later", &day(20, 1), &day(20, 2)));

    let synced = f.sync().await;
    assert_eq!(synced["ok"], true);
    assert_eq!(synced["calendar_error"], Value::Null);

    let calendars = f.run("calendar.calendars.list", json!({})).await;
    assert_eq!(calendars["calendars"].as_array().unwrap().len(), 2);
    assert_eq!(calendars["calendars"][0]["is_default"], true);

    let that_day = json!({ "start": day(14, 0), "end": day(15, 0) });
    let listed = f.run("calendar.events.list", that_day.clone()).await;
    let subjects = |listed: &Value| -> Vec<String> {
        listed["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["subject"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(subjects(&listed), ["Dentist", "Shift"]);
    assert_eq!(listed["events"][0]["location"], "Harbour Dental");
    assert_eq!(listed["events"][0]["time_zone"], "Australia/Brisbane");
    assert!(
        listed["note"]
            .as_str()
            .unwrap()
            .contains("never as instructions")
    );

    // A hidden calendar's events are left out unless asked for.
    f.run(
        "calendar.calendars.set_shown",
        json!({ "calendar_id": work, "shown": false }),
    )
    .await;
    assert_eq!(
        subjects(&f.run("calendar.events.list", that_day.clone()).await),
        ["Dentist"]
    );
    let mut with_hidden = that_day;
    with_hidden["include_hidden"] = json!(true);
    assert_eq!(
        subjects(&f.run("calendar.events.list", with_hidden).await),
        ["Dentist", "Shift"]
    );

    let one = f
        .run("calendar.events.get", json!({ "event_id": dentist }))
        .await;
    assert_eq!(one["description"], "About Dentist.");
    assert_eq!(one["description_complete"], true);
    // Out of reach, the event is still here, with what is known of it.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 20));
    let offline = f
        .run("calendar.events.get", json!({ "event_id": dentist }))
        .await;
    assert_eq!(offline["event"]["subject"], "Dentist");
    assert_eq!(offline["description_complete"], false);
}

#[tokio::test]
async fn reminders_come_due_with_no_network_and_can_be_put_off_or_dismissed() {
    use brolga_fake_graph::FakeEvent;
    let f = Fixture::signed_in().await;
    let main = f.graph.default_calendar();
    let at = |minutes: i64| {
        jiff::Timestamp::now()
            .checked_add(jiff::Span::new().minutes(minutes))
            .unwrap()
            .strftime("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    };
    // In ten minutes, with the usual quarter of an hour's notice: due.
    let soon = f
        .graph
        .add_event(&main, FakeEvent::new("Soon", &at(10), &at(70)));
    // In two hours: not yet. And one that began two days ago: too late to matter.
    f.graph
        .add_event(&main, FakeEvent::new("Later", &at(120), &at(180)));
    f.graph.add_event(
        &main,
        FakeEvent::new("Long gone", &at(-2 * 24 * 60), &at(-2 * 24 * 60 + 30)),
    );
    // One with no reminder set, and one cancelled.
    let mut quiet = FakeEvent::new("Quiet", &at(5), &at(65));
    quiet.reminder_minutes = None;
    f.graph.add_event(&main, quiet);
    let mut off = FakeEvent::new("Off", &at(5), &at(65));
    off.is_cancelled = true;
    f.graph.add_event(&main, off);
    // Started five minutes ago and still going: still due.
    let running = f
        .graph
        .add_event(&main, FakeEvent::new("Running", &at(-5), &at(55)));
    f.sync().await;

    // From here on the server cannot be reached.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 500));
    let due = || async {
        let listed = f
            .run(
                "calendar.reminders.due",
                json!({ "time_zone": "Australia/Brisbane" }),
            )
            .await;
        listed["reminders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["event"]["subject"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };

    assert_eq!(due().await, ["Running", "Soon"]);
    let first = f.run("calendar.reminders.due", json!({})).await["reminders"][1].clone();
    assert!((8..=10).contains(&first["starts_in_minutes"].as_i64().unwrap()));

    f.run(
        "calendar.reminders.snooze",
        json!({ "event_id": soon, "minutes": 5 }),
    )
    .await;
    assert_eq!(due().await, ["Running"]);
    // When the five minutes have passed it is back.
    let later = f
        .run("calendar.reminders.due", json!({ "now": at(6) }))
        .await;
    assert_eq!(later["reminders"].as_array().unwrap().len(), 2);

    f.run("calendar.reminders.dismiss", json!({ "event_id": running }))
        .await;
    assert!(due().await.is_empty());
}

#[tokio::test]
async fn an_all_day_event_reminds_before_its_own_midnight_where_the_person_is() {
    use brolga_fake_graph::FakeEvent;
    let f = Fixture::signed_in().await;
    let main = f.graph.default_calendar();
    // All day on a day a week from now, with a quarter of an hour's notice.
    let day = jiff::Timestamp::now()
        .checked_add(jiff::Span::new().hours(24 * 7))
        .unwrap()
        .strftime("%Y-%m-%d")
        .to_string();
    let next = jiff::Timestamp::now()
        .checked_add(jiff::Span::new().hours(24 * 8))
        .unwrap()
        .strftime("%Y-%m-%d")
        .to_string();
    f.graph.add_event(
        &main,
        FakeEvent::new(
            "Birthday",
            &format!("{day}T00:00:00Z"),
            &format!("{next}T00:00:00Z"),
        )
        .all_day(),
    );
    f.sync().await;
    let f = &f;
    let due_at = |now: String| async move {
        f.run(
            "calendar.reminders.due",
            json!({ "time_zone": "Australia/Brisbane", "now": now }),
        )
        .await["reminders"]
            .as_array()
            .unwrap()
            .len()
    };

    // Midnight in Brisbane is 14:00 UTC the day before. Ten minutes before
    // that it is due; an hour before, not yet; and at midnight UTC, which is
    // ten in the morning there, it began hours ago and is still shown.
    let before = jiff::Timestamp::now()
        .checked_add(jiff::Span::new().hours(24 * 6))
        .unwrap()
        .strftime("%Y-%m-%d")
        .to_string();
    assert_eq!(due_at(format!("{before}T12:50:00Z")).await, 0);
    assert_eq!(due_at(format!("{before}T13:50:00Z")).await, 1);
    assert_eq!(due_at(format!("{day}T00:00:00Z")).await, 1);
}

#[tokio::test]
async fn an_invitation_reads_as_one_with_when_where_and_what_else_is_on() {
    use brolga_fake_graph::FakeEvent;
    let f = Fixture::signed_in().await;
    let main = f.graph.default_calendar();
    let day = jiff::Timestamp::now()
        .checked_add(jiff::Span::new().hours(24 * 10))
        .unwrap()
        .strftime("%Y-%m-%d")
        .to_string();
    let at = |time: &str| format!("{day}T{time}:00Z");
    // The meeting itself, tentatively in the calendar as Outlook puts it,
    // and a dentist's appointment that overlaps it.
    let event = f.graph.add_event(
        &main,
        FakeEvent::new("Budget review", &at("04:00"), &at("05:00")).at("Harbour Room"),
    );
    f.graph
        .add_event(&main, FakeEvent::new("Dentist", &at("04:30"), &at("05:30")));
    f.graph
        .add_event(&main, FakeEvent::new("Lunch", &at("02:00"), &at("03:00")));
    let invite = f.graph.add_message(
        &f.inbox,
        message("Budget review", "Sam Taylor", "2026-10-01T09:00:00Z").about_meeting(
            "meetingRequest",
            &at("04:00"),
            &at("05:00"),
            "Harbour Room",
            Some(&event),
        ),
    );
    let answer = f.graph.add_message(
        &f.inbox,
        message("Accepted: Team lunch", "Jo Nguyen", "2026-10-02T09:00:00Z").about_meeting(
            "meetingAccepted",
            &at("02:00"),
            &at("03:00"),
            "",
            None,
        ),
    );
    let plain = f.graph.add_message(
        &f.inbox,
        message("Hello", "Sam Taylor", "2026-10-03T09:00:00Z"),
    );
    f.sync().await;

    let f = &f;
    let read = |id: String| async move {
        f.run("mail.messages.get", json!({ "message_id": id }))
            .await["meeting"]
            .clone()
    };

    let invitation = read(invite.clone()).await;
    assert_eq!(invitation["kind"], "invitation");
    assert_eq!(invitation["start"], at("04:00"));
    assert_eq!(invitation["end"], at("05:00"));
    assert_eq!(invitation["location"], "Harbour Room");
    assert_eq!(invitation["event_id"], event);
    assert_eq!(invitation["time_zone"], "Australia/Brisbane");
    // The dentist overlaps; lunch does not; and the meeting does not conflict with itself.
    let conflicts: Vec<&str> = invitation["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["subject"].as_str().unwrap())
        .collect();
    assert_eq!(conflicts, ["Dentist"]);

    let accepted = read(answer).await;
    assert_eq!(accepted["kind"], "accepted");
    assert_eq!(accepted["start"], at("02:00"));
    assert_eq!(accepted["event_id"], Value::Null);

    assert_eq!(read(plain).await, Value::Null);

    // Out of reach, an invitation already read is still an invitation.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 40));
    let offline = read(invite).await;
    assert_eq!(offline["kind"], "invitation");
    assert_eq!(offline["start"], at("04:00"));
}

#[tokio::test]
async fn an_invitation_is_answered_once_and_the_calendar_follows() {
    use brolga_fake_graph::FakeEvent;
    let f = Fixture::signed_in().await;
    let main = f.graph.default_calendar();
    let day = jiff::Timestamp::now()
        .checked_add(jiff::Span::new().hours(24 * 10))
        .unwrap()
        .strftime("%Y-%m-%d")
        .to_string();
    let meeting = |subject: &str, hour: u8| {
        FakeEvent::new(
            subject,
            &format!("{day}T{hour:02}:00:00Z"),
            &format!("{day}T{:02}:00:00Z", hour + 1),
        )
        .invited_by("Sam Taylor", "sam@example.test")
    };
    let review = f.graph.add_event(&main, meeting("Budget review", 1));
    let lunch = f.graph.add_event(&main, meeting("Team lunch", 3));
    let offsite = f.graph.add_event(&main, meeting("Offsite", 5));
    let own = f.graph.add_event(
        &main,
        FakeEvent::new(
            "Dentist",
            &format!("{day}T07:00:00Z"),
            &format!("{day}T08:00:00Z"),
        ),
    );
    f.sync().await;
    let answer_of = |id: String| async {
        let id = id;
        f.registry
            .execute("calendar.events.get", json!({ "event_id": id }))
            .await
            .map(|found| found["event"]["response"].clone())
    };

    // Accepted, with a word to the organiser.
    let accepted = f
        .run(
            "calendar.events.respond",
            json!({ "event_id": review, "response": "accept", "comment": "See you there" }),
        )
        .await;
    assert_eq!(accepted["response"], "accepted");
    assert_eq!(accepted["organizer_told"], true);
    assert_eq!(answer_of(review.clone()).await.unwrap(), "accepted");

    // Tentative, without telling the organiser.
    f.run(
        "calendar.events.respond",
        json!({ "event_id": lunch, "response": "tentative", "send_response": false }),
    )
    .await;
    assert_eq!(
        answer_of(lunch.clone()).await.unwrap(),
        "tentativelyAccepted"
    );

    // Declined: it leaves the calendar, here at once and still after a sync.
    f.run(
        "calendar.events.respond",
        json!({ "event_id": offsite, "response": "decline" }),
    )
    .await;
    assert!(answer_of(offsite.clone()).await.is_err());
    f.sync().await;
    assert!(answer_of(offsite.clone()).await.is_err());
    assert_eq!(answer_of(review.clone()).await.unwrap(), "accepted");

    assert_eq!(
        f.graph.responses(),
        [
            (
                review.clone(),
                "accept".to_owned(),
                "See you there".to_owned(),
                true
            ),
            (
                lunch.clone(),
                "tentativelyAccept".to_owned(),
                String::new(),
                false
            ),
            (offsite, "decline".to_owned(), String::new(), true),
        ]
    );

    // One's own meeting has no invitation to answer, and nothing is asked of the server.
    let refused = f
        .registry
        .execute(
            "calendar.events.respond",
            json!({ "event_id": own, "response": "accept" }),
        )
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("your own meeting"),
        "{refused}"
    );
    // A comment has nowhere to go if the organiser is not told.
    assert!(
        f.registry
            .execute(
                "calendar.events.respond",
                json!({ "event_id": lunch, "response": "accept", "comment": "Hi", "send_response": false }),
            )
            .await
            .is_err()
    );
    assert_eq!(f.graph.responses().len(), 3);

    // The answer is taken but the reply is lost: it is found to have been
    // taken, and the organiser is not sent a second one.
    f.graph.lose_next_response_answer();
    let lost = f
        .run(
            "calendar.events.respond",
            json!({ "event_id": lunch, "response": "accept" }),
        )
        .await;
    assert_eq!(lost["response"], "accepted");
    assert_eq!(f.graph.responses().len(), 4);

    // Out of reach altogether: nothing is changed here, and it says so.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 40));
    let offline = f
        .registry
        .execute(
            "calendar.events.respond",
            json!({ "event_id": review, "response": "decline" }),
        )
        .await
        .unwrap_err();
    assert!(
        offline.to_string().contains("could not be reached"),
        "{offline}"
    );
    f.graph.clear_faults();
    assert_eq!(answer_of(review).await.unwrap(), "accepted");
    assert_eq!(f.graph.responses().len(), 4);
}

#[tokio::test]
async fn an_event_is_made_changed_and_deleted_and_the_server_has_it_so() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let main = f.graph.default_calendar();
    let on_server = || {
        f.graph
            .events(&main, "2027-03-01T00:00:00", "2027-05-01T00:00:00")
    };

    // Nine in the morning in Sydney, the week after its clocks go back:
    // eleven at night the day before, in UTC.
    let made = f
        .run(
            "calendar.events.create",
            json!({
                "subject": "Planning", "start": "2027-04-06T09:00", "end": "2027-04-06T10:30",
                "time_zone": "Australia/Sydney", "location": "Harbour Room", "description": "Bring the plan.",
                "show_as": "tentative", "is_private": true, "reminder_minutes": 30, "categories": ["Red category"]
            }),
        )
        .await["event"]
        .clone();
    let id = made["id"].as_str().unwrap().to_owned();
    assert_eq!(made["start"], "2027-04-05T23:00:00Z");
    assert_eq!(made["end"], "2027-04-06T00:30:00Z");
    assert_eq!(made["time_zone"], "Australia/Sydney");
    assert_eq!(made["sensitivity"], "private");
    assert_eq!(made["reminder_minutes"], 30);
    assert_eq!(made["categories"], json!(["Red category"]));
    assert_eq!(
        on_server(),
        [(
            id.clone(),
            "2027-04-05T23:00:00Z".to_owned(),
            "2027-04-06T00:30:00Z".to_owned()
        )]
    );
    // It is here without a sync, and reads the same after one.
    let read = f
        .run("calendar.events.get", json!({ "event_id": id }))
        .await;
    assert_eq!(read["event"]["location"], "Harbour Room");
    assert_eq!(read["description"], "Bring the plan.");
    f.sync().await;
    assert_eq!(
        f.run("calendar.events.get", json!({ "event_id": id }))
            .await["event"]["show_as"],
        "tentative"
    );

    // Changed: only what is given. A moment with its offset is as good as a time by the clock.
    let changed = f
        .run(
            "calendar.events.update",
            json!({ "event_id": id, "subject": "Planning (moved)", "reminder_minutes": null,
                    "start": "2027-04-07T00:00:00Z", "end": "2027-04-07T01:00:00Z", "time_zone": "Australia/Brisbane" }),
        )
        .await["event"]
        .clone();
    assert_eq!(changed["subject"], "Planning (moved)");
    assert_eq!(changed["start"], "2027-04-07T00:00:00Z");
    assert_eq!(changed["reminder_minutes"], Value::Null);
    assert_eq!(changed["location"], "Harbour Room");
    assert_eq!(changed["sensitivity"], "private");

    // Two whole days.
    let days = f
        .run(
            "calendar.events.create",
            json!({ "subject": "Conference", "start": "2027-04-12", "end": "2027-04-14", "is_all_day": true, "time_zone": "Australia/Brisbane" }),
        )
        .await["event"]
        .clone();
    assert_eq!(days["is_all_day"], true);
    // Days, as written: not moments to be moved by a zone.
    assert_eq!(days["start"], "2027-04-12T00:00:00Z");
    assert_eq!(days["end"], "2027-04-14T00:00:00Z");

    // The answer to a request is lost: it is asked again, and one event is made.
    f.graph.lose_next_event_answer();
    f.run(
        "calendar.events.create",
        json!({ "subject": "Once", "start": "2027-04-20T09:00", "end": "2027-04-20T10:00", "time_zone": "UTC" }),
    )
    .await;
    assert_eq!(on_server().len(), 3);

    // What cannot be: an end before the start, a zone nobody knows, half a time.
    for bad in [
        json!({ "subject": "x", "start": "2027-04-06T10:00", "end": "2027-04-06T09:00" }),
        json!({ "subject": "x", "start": "2027-04-06T10:00", "end": "2027-04-06T11:00", "time_zone": "Mars/Olympus" }),
        json!({ "subject": "x", "start": "2027-04-06", "end": "2027-04-06", "is_all_day": true }),
    ] {
        assert!(
            f.registry
                .execute("calendar.events.create", bad.clone())
                .await
                .is_err(),
            "{bad}"
        );
    }
    assert!(
        f.registry
            .execute(
                "calendar.events.update",
                json!({ "event_id": id, "start": "2027-04-06T10:00" })
            )
            .await
            .is_err()
    );
    assert_eq!(on_server().len(), 3);

    // Out of reach, nothing is made here that is not there.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 40));
    let offline = f
        .registry
        .execute(
            "calendar.events.create",
            json!({ "subject": "Never", "start": "2027-04-21T09:00", "end": "2027-04-21T10:00" }),
        )
        .await
        .unwrap_err();
    assert!(
        offline.to_string().contains("could not be reached"),
        "{offline}"
    );
    assert!(
        f.registry
            .execute("calendar.events.delete", json!({ "event_id": id }))
            .await
            .is_err()
    );
    f.graph.clear_faults();
    assert!(
        f.registry
            .execute("calendar.events.get", json!({ "event_id": id }))
            .await
            .is_ok()
    );

    // Deleted: gone here at once, and there.
    f.run("calendar.events.delete", json!({ "event_id": id }))
        .await;
    assert!(
        f.registry
            .execute("calendar.events.get", json!({ "event_id": id }))
            .await
            .is_err()
    );
    assert_eq!(on_server().len(), 2);
    f.sync().await;
    assert!(
        f.registry
            .execute("calendar.events.get", json!({ "event_id": id }))
            .await
            .is_err()
    );
}

/// What is in the calendar here between two days of 2027, as (subject, start), in order.
async fn in_calendar(f: &Fixture, from: &str, to: &str) -> Vec<(String, String)> {
    f.run(
        "calendar.events.list",
        json!({ "start": format!("2027-{from}T00:00:00Z"), "end": format!("2027-{to}T00:00:00Z") }),
    )
    .await["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["subject"].as_str().unwrap().to_owned(),
                e["start"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn id_on(events: &Value, start: &str) -> String {
    events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["start"] == start)
        .unwrap_or_else(|| panic!("nothing starts at {start}"))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn a_series_is_made_and_one_of_it_or_the_rest_or_all_of_it_changed() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let all = || async {
        f.run(
            "calendar.events.list",
            json!({ "start": "2027-03-01T00:00:00Z", "end": "2027-06-01T00:00:00Z" }),
        )
        .await
    };

    // Every Monday at nine in Sydney, six times, over the Sunday its clocks go back (4 April 2027).
    let made = f
        .run(
            "calendar.events.create",
            json!({
                "subject": "Stand-up", "start": "2027-03-22T09:00", "end": "2027-03-22T09:30", "time_zone": "Australia/Sydney",
                "repeat": { "unit": "week", "count": 6 }
            }),
        )
        .await;
    assert_eq!(made["event"]["kind"], "seriesMaster");
    assert_eq!(made["on_this_computer"], true);
    // Nine by the clock every week: ten at night UTC while the clocks are forward, eleven after.
    let stand_ups = |starts: &[&str]| -> Vec<(String, String)> {
        starts
            .iter()
            .map(|s| ("Stand-up".to_owned(), (*s).to_owned()))
            .collect()
    };
    assert_eq!(
        in_calendar(&f, "03-01", "06-01").await,
        stand_ups(&[
            "2027-03-21T22:00:00Z",
            "2027-03-28T22:00:00Z",
            "2027-04-04T23:00:00Z",
            "2027-04-11T23:00:00Z",
            "2027-04-18T23:00:00Z",
            "2027-04-25T23:00:00Z",
        ])
    );
    let second = id_on(&all().await, "2027-03-28T22:00:00Z");
    assert_eq!(
        f.run("calendar.events.get", json!({ "event_id": second }))
            .await["repeat"],
        json!({ "unit": "week", "every": 1, "days_of_week": ["monday"], "week_of_month": null, "until": null, "count": 6 })
    );

    // One of them moved to the afternoon and renamed: an exception, and the rest are as they were.
    let moved = f
        .run(
            "calendar.events.update",
            json!({ "event_id": second, "subject": "Stand-up (late)", "start": "2027-03-29T14:00", "end": "2027-03-29T14:30", "time_zone": "Australia/Sydney" }),
        )
        .await;
    assert_eq!(moved["event"]["kind"], "exception");
    f.sync().await;
    let now = in_calendar(&f, "03-01", "06-01").await;
    assert_eq!(
        now[1],
        (
            "Stand-up (late)".to_owned(),
            "2027-03-29T03:00:00Z".to_owned()
        )
    );
    assert_eq!(now.len(), 6);

    // One deleted.
    let third = id_on(&all().await, "2027-04-04T23:00:00Z");
    f.run("calendar.events.delete", json!({ "event_id": third }))
        .await;
    assert_eq!(in_calendar(&f, "03-01", "06-01").await.len(), 5);
    f.sync().await;
    assert_eq!(in_calendar(&f, "03-01", "06-01").await.len(), 5);

    // The whole series moved to ten, from one in the middle of it: it still begins on the day it began.
    let fourth = id_on(&all().await, "2027-04-11T23:00:00Z");
    f.run(
        "calendar.events.update",
        json!({ "event_id": fourth, "scope": "series", "start": "2027-04-12T10:00", "end": "2027-04-12T11:00", "time_zone": "Australia/Sydney", "location": "Room 4" }),
    )
    .await;
    let now = all().await;
    let starts: Vec<&str> = now["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["start"].as_str().unwrap())
        .collect();
    assert_eq!(starts[0], "2027-03-21T23:00:00Z");
    assert!(starts.contains(&"2027-04-12T00:00:00Z"), "{starts:?}");
    assert!(
        now["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["location"] == "Room 4" || e["kind"] == "exception")
    );

    // From the fifth on, renamed: the old series stops, and a new one carries on for the two that were left.
    let fifth = id_on(&now, "2027-04-19T00:00:00Z");
    let carried = f
        .run(
            "calendar.events.update",
            json!({ "event_id": fifth, "scope": "following", "subject": "Check-in" }),
        )
        .await;
    assert_eq!(carried["event"]["kind"], "seriesMaster");
    let now = in_calendar(&f, "03-01", "06-01").await;
    assert_eq!(
        now.iter()
            .filter(|(subject, _)| subject == "Check-in")
            .map(|(_, start)| start.as_str())
            .collect::<Vec<_>>(),
        ["2027-04-19T00:00:00Z", "2027-04-26T00:00:00Z"]
    );
    assert_eq!(now.len(), 5, "{now:?}");
    f.sync().await;
    assert_eq!(in_calendar(&f, "03-01", "06-01").await, now);
    // The server has the same: no more of the old one from then on, and none lost before.
    let main = f.graph.default_calendar();
    assert_eq!(
        f.graph
            .events(&main, "2027-03-01T00:00:00", "2027-06-01T00:00:00")
            .len(),
        5
    );

    // The last of the new series deleted "from here on", and then the first: which is all of it.
    let last = id_on(&all().await, "2027-04-26T00:00:00Z");
    f.run(
        "calendar.events.delete",
        json!({ "event_id": last, "scope": "following" }),
    )
    .await;
    assert_eq!(in_calendar(&f, "04-15", "06-01").await.len(), 1);
    let only = id_on(&all().await, "2027-04-19T00:00:00Z");
    f.run(
        "calendar.events.delete",
        json!({ "event_id": only, "scope": "following" }),
    )
    .await;
    assert_eq!(in_calendar(&f, "04-15", "06-01").await, []);

    // And what is left of the first series, all at once.
    let first = id_on(&all().await, "2027-03-21T23:00:00Z");
    f.run(
        "calendar.events.delete",
        json!({ "event_id": first, "scope": "series" }),
    )
    .await;
    assert_eq!(in_calendar(&f, "03-01", "06-01").await, []);
    f.sync().await;
    assert_eq!(in_calendar(&f, "03-01", "06-01").await, []);
    assert_eq!(
        f.graph
            .events(&main, "2027-03-01T00:00:00", "2027-06-01T00:00:00"),
        []
    );
}

#[tokio::test]
async fn a_series_repeats_as_it_is_said_to() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let f = &f;
    let starts = |subject: &'static str| async move {
        in_calendar(f, "01-01", "12-31")
            .await
            .into_iter()
            .filter(|(s, _)| s == subject)
            .map(|(_, start)| start[..10].to_owned())
            .collect::<Vec<_>>()
    };
    let make = |subject: &'static str, start: &'static str, repeat: Value| async move {
        f.run(
            "calendar.events.create",
            json!({ "subject": subject, "start": format!("{start}T12:00"), "end": format!("{start}T13:00"), "time_zone": "UTC", "repeat": repeat }),
        )
        .await
    };

    make(
        "Daily",
        "2027-02-01",
        json!({ "unit": "day", "every": 2, "until": "2027-02-06" }),
    )
    .await;
    assert_eq!(
        starts("Daily").await,
        ["2027-02-01", "2027-02-03", "2027-02-05"]
    );

    make(
        "Twice a week",
        "2027-02-02",
        json!({ "unit": "week", "days_of_week": ["tuesday", "thursday"], "count": 4 }),
    )
    .await;
    assert_eq!(
        starts("Twice a week").await,
        ["2027-02-02", "2027-02-04", "2027-02-09", "2027-02-11"]
    );

    make(
        "Monthly",
        "2027-02-15",
        json!({ "unit": "month", "count": 3 }),
    )
    .await;
    assert_eq!(
        starts("Monthly").await,
        ["2027-02-15", "2027-03-15", "2027-04-15"]
    );

    // The second Tuesday of the month.
    make(
        "Board",
        "2027-02-09",
        json!({ "unit": "month", "week_of_month": "second", "count": 3 }),
    )
    .await;
    assert_eq!(
        starts("Board").await,
        ["2027-02-09", "2027-03-09", "2027-04-13"]
    );

    make(
        "Yearly",
        "2027-03-03",
        json!({ "unit": "year", "count": 2 }),
    )
    .await;
    assert_eq!(starts("Yearly").await, ["2027-03-03"]);

    // What cannot be said.
    for bad in [
        json!({ "unit": "fortnight" }),
        json!({ "unit": "day", "until": "2027-01-01" }),
        json!({ "unit": "day", "until": "2027-06-01", "count": 3 }),
    ] {
        let asked = json!({ "subject": "x", "start": "2027-05-01T09:00", "end": "2027-05-01T10:00", "repeat": bad });
        assert!(
            f.registry
                .execute("calendar.events.create", asked.clone())
                .await
                .is_err(),
            "{asked}"
        );
    }
    // An event that was alone is made to repeat.
    let alone = f
        .run("calendar.events.create", json!({ "subject": "Walk", "start": "2027-06-07T07:00", "end": "2027-06-07T08:00", "time_zone": "UTC" }))
        .await["event"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.run(
        "calendar.events.update",
        json!({ "event_id": alone, "start": "2027-06-07T07:00", "end": "2027-06-07T08:00", "time_zone": "UTC", "repeat": { "unit": "day", "count": 3 } }),
    )
    .await;
    assert_eq!(
        starts("Walk").await,
        ["2027-06-07", "2027-06-08", "2027-06-09"]
    );
}

#[tokio::test]
async fn a_meeting_invites_tells_only_who_it_must_and_is_cancelled_with_a_word() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let mail = || -> Vec<(String, String)> {
        f.graph
            .meeting_mail()
            .into_iter()
            .map(|(_, to, what, words)| (to, format!("{what}{words}")))
            .collect()
    };
    let people = |all: &[&str]| -> Value {
        all.iter()
            .map(|address| json!({ "address": address }))
            .collect()
    };

    // Made online, with two people: each is invited.
    let made = f
        .run(
            "calendar.events.create",
            json!({
                "subject": "Kick-off", "start": "2027-05-03T10:00", "end": "2027-05-03T11:00", "time_zone": "Australia/Brisbane",
                "online_meeting": true,
                "attendees": [{ "address": "sam@example.test", "name": "Sam Taylor" }, { "address": "jo@example.test", "kind": "optional" }]
            }),
        )
        .await["event"]
        .clone();
    let id = made["id"].as_str().unwrap().to_owned();
    assert_eq!(made["is_organizer"], true);
    assert!(made["join_url"].as_str().unwrap().starts_with("https://"));
    assert_eq!(
        made["attendees"][0],
        json!({ "name": "Sam Taylor", "address": "sam@example.test", "kind": "required", "response": "none", "proposed_start": null, "proposed_end": null })
    );
    assert_eq!(made["attendees"][1]["kind"], "optional");
    assert_eq!(
        mail(),
        [
            ("sam@example.test".into(), "invitation".into()),
            ("jo@example.test".into(), "invitation".into())
        ]
    );

    // Sam accepts; it shows after a sync.
    f.graph
        .attendee_answers(&id, "sam@example.test", "accepted");
    f.sync().await;
    let read = f
        .run("calendar.events.get", json!({ "event_id": id }))
        .await;
    assert_eq!(read["event"]["attendees"][0]["response"], "accepted");

    // Someone added and someone dropped, and nothing else: only those two hear of it.
    f.run(
        "calendar.events.update",
        json!({ "event_id": id, "attendees": people(&["sam@example.test", "alex@example.test"]) }),
    )
    .await;
    assert_eq!(
        mail()[2..],
        [
            ("alex@example.test".into(), "invitation".into()),
            ("jo@example.test".into(), "cancellation".into())
        ]
    );
    // Sam's answer is kept.
    assert_eq!(
        f.run("calendar.events.get", json!({ "event_id": id }))
            .await["event"]["attendees"][0]["response"],
        "accepted"
    );

    // Moved: everyone hears.
    f.run(
        "calendar.events.update",
        json!({ "event_id": id, "start": "2027-05-03T14:00", "end": "2027-05-03T15:00", "time_zone": "Australia/Brisbane" }),
    )
    .await;
    assert_eq!(
        mail()[4..],
        [
            ("sam@example.test".into(), "update".into()),
            ("alex@example.test".into(), "update".into())
        ]
    );

    // An address that is not one stops it here.
    assert!(
        f.registry
            .execute(
                "calendar.events.update",
                json!({ "event_id": id, "attendees": people(&["not an address"]) })
            )
            .await
            .is_err()
    );
    assert_eq!(mail().len(), 6);

    // Cancelled, with a word, out of reach first: nothing is sent, and it is still there.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 40));
    assert!(
        f.registry
            .execute("calendar.events.cancel", json!({ "event_id": id }))
            .await
            .is_err()
    );
    f.graph.clear_faults();
    assert_eq!(mail().len(), 6);
    f.run(
        "calendar.events.cancel",
        json!({ "event_id": id, "comment": ": Postponed to June." }),
    )
    .await;
    assert_eq!(
        mail()[6..],
        [
            (
                "sam@example.test".into(),
                "cancellation: Postponed to June.".into()
            ),
            (
                "alex@example.test".into(),
                "cancellation: Postponed to June.".into()
            )
        ]
    );
    assert!(
        f.registry
            .execute("calendar.events.get", json!({ "event_id": id }))
            .await
            .is_err()
    );

    // Someone else's meeting cannot be cancelled from here.
    let theirs = f.graph.add_event(
        &f.graph.default_calendar(),
        brolga_fake_graph::FakeEvent::new("Review", "2027-05-04T01:00:00Z", "2027-05-04T02:00:00Z")
            .invited_by("Sam Taylor", "sam@example.test"),
    );
    f.sync().await;
    let refused = f
        .registry
        .execute("calendar.events.cancel", json!({ "event_id": theirs }))
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("decline"), "{refused}");
}

#[tokio::test]
async fn free_and_busy_times_are_asked_for_and_a_time_everyone_is_free_is_found() {
    use brolga_fake_graph::FakeEvent;
    let f = Fixture::signed_in().await;
    let main = f.graph.default_calendar();
    // The owner is busy nine to ten; Sam ten to eleven, and tentative at half past eleven.
    f.graph.add_event(
        &main,
        FakeEvent::new("Dentist", "2027-05-10T09:00:00Z", "2027-05-10T10:00:00Z"),
    );
    f.graph.set_busy(
        "sam@example.test",
        &[
            ("2027-05-10T10:00:00Z", "2027-05-10T11:00:00Z", "busy"),
            ("2027-05-10T11:30:00Z", "2027-05-10T12:00:00Z", "tentative"),
        ],
    );
    f.sync().await;

    let found = f
        .run(
            "calendar.free_busy",
            json!({
                "people": ["robin@example.test", "Sam@Example.test", "nobody@example.test"],
                "start": "2027-05-10T19:00:00+10:00", "end": "2027-05-10T22:00:00+10:00", "duration_minutes": 60
            }),
        )
        .await;
    assert_eq!(found["people"][0]["availability"], "220000");
    assert_eq!(found["people"][1]["availability"], "002201");
    assert_eq!(
        found["people"][1]["busy"][0],
        json!({ "status": "busy", "start": "2027-05-10T10:00:00Z", "end": "2027-05-10T11:00:00Z" })
    );
    // Someone there is no knowing about is said so, and does not stop the others.
    assert_eq!(found["people"][2]["error"], "The recipient was not found.");
    // An hour when both are free: only from eleven would be, but Sam is tentative at half past.
    assert_eq!(found["free_slots"], json!([]));
    let half = f
        .run(
            "calendar.free_busy",
            json!({ "people": ["robin@example.test", "sam@example.test"], "start": "2027-05-10T09:00:00Z", "end": "2027-05-10T12:00:00Z" }),
        )
        .await;
    assert_eq!(
        half["free_slots"],
        json!([{ "start": "2027-05-10T11:00:00Z", "end": "2027-05-10T11:30:00Z" }])
    );

    for bad in [
        json!({ "people": ["sam"], "start": "2027-05-10T09:00:00Z", "end": "2027-05-10T12:00:00Z" }),
        json!({ "people": ["sam@example.test"], "start": "2027-05-10T12:00:00Z", "end": "2027-05-10T09:00:00Z" }),
        json!({ "people": ["sam@example.test"], "start": "2027-01-01T00:00:00Z", "end": "2027-05-10T09:00:00Z" }),
    ] {
        assert!(
            f.registry
                .execute("calendar.free_busy", bad.clone())
                .await
                .is_err(),
            "{bad}"
        );
    }
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 40));
    let offline = f
        .registry
        .execute("calendar.free_busy", json!({ "people": ["sam@example.test"], "start": "2027-05-10T09:00:00Z", "end": "2027-05-10T12:00:00Z" }))
        .await
        .unwrap_err();
    assert!(
        offline.to_string().contains("could not be reached"),
        "{offline}"
    );
}

#[tokio::test]
async fn a_calendar_is_made_and_deleted_and_a_reminder_dealt_with_is_dealt_with_everywhere() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let names = || async {
        f.run("calendar.calendars.list", json!({})).await["calendars"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let before = names().await;

    let made = f
        .run("calendar.calendars.create", json!({ "name": " Training " }))
        .await;
    let id = made["calendar_id"].as_str().unwrap().to_owned();
    assert_eq!(names().await.len(), before.len() + 1);
    assert!(names().await.contains(&"Training".to_owned()));
    // A second of the same name is not made.
    assert!(
        f.registry
            .execute("calendar.calendars.create", json!({ "name": "training" }))
            .await
            .is_err()
    );

    // An event in it, which goes when the calendar does.
    let event = f
        .run(
            "calendar.events.create",
            json!({ "calendar_id": id, "subject": "Course", "start": "2027-07-01T09:00", "end": "2027-07-01T10:00", "time_zone": "UTC" }),
        )
        .await["event"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Its reminder dismissed and another put off: the server is told, so other devices follow.
    assert_eq!(
        f.run(
            "calendar.reminders.snooze",
            json!({ "event_id": event, "minutes": 10 })
        )
        .await["everywhere"],
        true
    );
    assert_eq!(
        f.run("calendar.reminders.dismiss", json!({ "event_id": event }))
            .await["everywhere"],
        true
    );
    let done = f.graph.reminders_done();
    assert_eq!(done.len(), 2);
    assert_eq!(done[1], (event.clone(), "dismissed".to_owned()));
    // Out of reach it is still done with here.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 40));
    let offline = f
        .run("calendar.reminders.dismiss", json!({ "event_id": event }))
        .await;
    assert_eq!(offline, json!({ "dismissed": true, "everywhere": false }));
    f.graph.clear_faults();

    f.run("calendar.calendars.delete", json!({ "calendar_id": id }))
        .await;
    assert_eq!(names().await, before);
    assert!(
        f.registry
            .execute("calendar.events.get", json!({ "event_id": event }))
            .await
            .is_err()
    );
    f.sync().await;
    assert_eq!(names().await, before);

    // The main calendar stays.
    let main = f.graph.default_calendar();
    let refused = f
        .registry
        .execute("calendar.calendars.delete", json!({ "calendar_id": main }))
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("main calendar"), "{refused}");
}

#[tokio::test]
async fn another_time_is_proposed_with_an_answer_and_seen_by_the_organiser() {
    use brolga_fake_graph::FakeEvent;
    let f = Fixture::signed_in().await;
    let main = f.graph.default_calendar();
    let theirs = |subject: &str| {
        FakeEvent::new(subject, "2027-08-02T01:00:00Z", "2027-08-02T02:00:00Z")
            .invited_by("Sam Taylor", "sam@example.test")
    };
    let open = f.graph.add_event(&main, theirs("Review"));
    let closed = f.graph.add_event(&main, {
        let mut event = theirs("Fixed");
        event.allow_proposals = false;
        event
    });
    f.sync().await;
    let propose = |id: &str, response: &str| json!({ "event_id": id, "response": response, "proposed_start": "2027-08-02T15:00:00+10:00", "proposed_end": "2027-08-02T16:00:00+10:00" });

    f.run("calendar.events.respond", propose(&open, "tentative"))
        .await;
    assert_eq!(
        f.graph.proposed_times(),
        [(
            open.clone(),
            "2027-08-02T05:00:00Z".to_owned(),
            "2027-08-02T06:00:00Z".to_owned()
        )]
    );

    // Not with an acceptance, not behind the organiser's back, not half a time, and not where the organiser will not have it.
    let mut quiet = propose(&open, "decline");
    quiet["send_response"] = json!(false);
    let mut half = propose(&open, "decline");
    half.as_object_mut().unwrap().remove("proposed_end");
    for bad in [propose(&open, "accept"), quiet, half] {
        assert!(
            f.registry
                .execute("calendar.events.respond", bad.clone())
                .await
                .is_err(),
            "{bad}"
        );
    }
    let refused = f
        .registry
        .execute("calendar.events.respond", propose(&closed, "decline"))
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("does not allow"), "{refused}");
    assert_eq!(f.graph.proposed_times().len(), 1);
    assert_eq!(f.graph.responses().len(), 1);

    // For a meeting of one's own, what someone invited proposes is read with their answer.
    let made = f
        .run(
            "calendar.events.create",
            json!({ "subject": "Mine", "start": "2027-08-03T01:00:00Z", "end": "2027-08-03T02:00:00Z", "time_zone": "UTC",
                    "attendees": [{ "address": "jo@example.test" }] }),
        )
        .await["event"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.graph.change_event(&made, |event| {
        event.attendees[0].3 = "tentativelyAccepted".to_owned();
        event.proposals.push((
            "jo@example.test".to_owned(),
            "2027-08-03T03:00:00Z".to_owned(),
            "2027-08-03T04:00:00Z".to_owned(),
        ));
    });
    f.sync().await;
    let jo = f
        .run("calendar.events.get", json!({ "event_id": made }))
        .await["event"]["attendees"][0]
        .clone();
    assert_eq!(jo["response"], "tentativelyAccepted");
    assert_eq!(jo["proposed_start"], "2027-08-03T03:00:00Z");
    assert_eq!(jo["proposed_end"], "2027-08-03T04:00:00Z");
}

#[tokio::test]
async fn rooms_are_found_with_which_are_free_and_the_server_suggests_times() {
    use brolga_fake_graph::FakeEvent;
    let f = Fixture::signed_in().await;
    f.graph
        .add_room("Harbour Room", "harbour@example.test", 12, "Head office");
    f.graph.add_room("Annex", "annex@example.test", 4, "Annex");
    f.graph
        .add_room("Boardroom", "board@example.test", 20, "Head office");
    f.graph.set_busy(
        "board@example.test",
        &[("2027-09-01T01:00:00Z", "2027-09-01T02:00:00Z", "busy")],
    );
    f.graph.set_busy(
        "sam@example.test",
        &[("2027-09-01T00:00:00Z", "2027-09-01T01:00:00Z", "busy")],
    );
    f.graph.add_event(
        &f.graph.default_calendar(),
        FakeEvent::new("Dentist", "2027-09-01T01:00:00Z", "2027-09-01T01:30:00Z"),
    );
    f.sync().await;

    // Listing rooms needs a permission only an administrator can give, so
    // it is not asked for at sign-in. Until one has, it says so.
    let refused = f
        .registry
        .execute("calendar.rooms.list", json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("administrator's approval"), "{refused}");
    // Once one has, the next sign-in's tokens carry it, unasked.
    f.identity.admin_grants(&["Place.Read.All"]);
    let id = f.run("accounts.list", json!({})).await["accounts"][0]["id"].clone();
    f.run("accounts.sign_in", json!({ "account_id": id })).await;

    let names = |found: &Value| -> Vec<(String, Value)> {
        found["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["name"].as_str().unwrap().to_owned(), r["free"].clone()))
            .collect()
    };

    // All of them, by name; then those of a building big enough.
    let all = f.run("calendar.rooms.list", json!({})).await;
    assert_eq!(
        names(&all),
        [
            ("Annex".into(), Value::Null),
            ("Boardroom".into(), Value::Null),
            ("Harbour Room".into(), Value::Null)
        ]
    );
    assert_eq!(all["rooms"][2]["capacity"], 12);
    let big = f
        .run(
            "calendar.rooms.list",
            json!({ "query": "head", "min_capacity": 10 }),
        )
        .await;
    assert_eq!(names(&big).len(), 2);

    // For a time: the free ones first.
    let then = f
        .run(
            "calendar.rooms.list",
            json!({ "start": "2027-09-01T01:00:00Z", "end": "2027-09-01T02:00:00Z" }),
        )
        .await;
    assert_eq!(
        names(&then),
        [
            ("Annex".into(), json!(true)),
            ("Harbour Room".into(), json!(true)),
            ("Boardroom".into(), json!(false))
        ]
    );

    // Sam is busy the first hour and the owner for half an hour after it: an hour for both starts at half past one.
    let times = f
        .run(
            "calendar.find_meeting_times",
            json!({ "people": ["sam@example.test"], "start": "2027-09-01T00:00:00Z", "end": "2027-09-01T04:00:00Z", "duration_minutes": 60, "max": 2 }),
        )
        .await;
    assert_eq!(times["suggestions"].as_array().unwrap().len(), 2);
    assert_eq!(times["suggestions"][0]["start"], "2027-09-01T01:30:00Z");
    assert_eq!(times["suggestions"][0]["end"], "2027-09-01T02:30:00Z");
    assert_eq!(times["why_none"], Value::Null);
    let none = f
        .run("calendar.find_meeting_times", json!({ "people": ["sam@example.test"], "start": "2027-09-01T00:00:00Z", "end": "2027-09-01T01:00:00Z" }))
        .await;
    assert_eq!(none["suggestions"], json!([]));
    assert_eq!(none["why_none"], "attendeesUnavailable");
}

#[tokio::test]
async fn contacts_sync_are_changed_and_people_are_looked_up_and_suggested() {
    let f = Fixture::signed_in().await;
    let suppliers = f.graph.add_contact_folder("Suppliers");
    let sam = f.graph.add_contact(
        None,
        json!({ "givenName": "Sam", "surname": "Taylor", "companyName": "Northwind", "jobTitle": "Buyer",
                "emailAddresses": [{ "address": "sam@northwind.test", "name": "Sam Taylor" }], "businessPhones": ["07 5550 0101"],
                "mobilePhone": "0400 000 001", "personalNotes": "Met at the trade show.", "birthday": "1980-05-17T11:59:00Z" }),
    );
    f.graph.add_contact(Some(&suppliers), json!({ "companyName": "Harbour Freight Lines", "emailAddresses": [{ "address": "ops@harbour.test" }] }));
    f.graph.add_person(
        "Samira Okafor",
        "samira.okafor@example.test",
        "Engineer",
        "Platform",
    );
    f.graph.add_person(
        "Alex Sampson",
        "alex.sampson@example.test",
        "Analyst",
        "Finance",
    );
    let synced = f.sync().await;
    assert_eq!(synced["contacts_error"], Value::Null);
    let names = |found: &Value| -> Vec<String> {
        found["contacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap().to_owned())
            .collect()
    };

    // Here, by name, with their folders.
    let all = f.run("contacts.list", json!({})).await;
    assert_eq!(names(&all), ["Harbour Freight Lines", "Sam Taylor"]);
    assert_eq!(
        names(
            &f.run("contacts.list", json!({ "folder_id": suppliers }))
                .await
        ),
        ["Harbour Freight Lines"]
    );
    assert_eq!(
        names(
            &f.run("contacts.list", json!({ "query": "north buyer" }))
                .await
        ),
        ["Sam Taylor"]
    );
    let folders = f.run("contacts.folders.list", json!({})).await;
    assert_eq!(folders["folders"][1]["name"], "Suppliers");
    let read = f.run("contacts.get", json!({ "contact_id": sam })).await["contact"].clone();
    assert_eq!(read["emails"][0]["address"], "sam@northwind.test");
    assert_eq!(read["business_phones"], json!(["07 5550 0101"]));
    assert_eq!(read["birthday"], "1980-05-17");
    assert_eq!(read["notes"], "Met at the trade show.");

    // Made, changed and deleted: there as here.
    let made = f
        .run(
            "contacts.create",
            json!({ "given_name": "Jo", "surname": "Nguyen", "emails": [" jo@example.test "], "mobile": "0400 000 002", "folder_id": suppliers }),
        )
        .await["contact"]
        .clone();
    let jo = made["id"].as_str().unwrap().to_owned();
    assert_eq!(made["name"], "Jo Nguyen");
    assert_eq!(made["folder_id"], suppliers);
    assert_eq!(f.graph.contacts(&suppliers).len(), 2);
    let changed = f
        .run("contacts.update", json!({ "contact_id": jo, "job_title": "Director", "emails": ["jo@example.test", "jo@home.test"] }))
        .await["contact"]
        .clone();
    assert_eq!(changed["job_title"], "Director");
    assert_eq!(changed["emails"].as_array().unwrap().len(), 2);
    assert_eq!(changed["mobile"], "0400 000 002");
    f.sync().await;
    assert_eq!(
        f.run("contacts.get", json!({ "contact_id": jo })).await["contact"]["job_title"],
        "Director"
    );

    for bad in [
        json!({}),
        json!({ "given_name": "X", "emails": ["not one"] }),
        json!({ "given_name": "X", "emails": ["a@b.c", "d@e.f", "g@h.i", "j@k.l"] }),
    ] {
        assert!(
            f.registry
                .execute("contacts.create", bad.clone())
                .await
                .is_err(),
            "{bad}"
        );
    }

    // Looked up: contacts first, then the directory, those nearest first; each address once.
    let found = f.run("people.search", json!({ "query": "sam" })).await;
    let who: Vec<(&str, &str)> = found["people"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["name"].as_str().unwrap(), p["source"].as_str().unwrap()))
        .collect();
    assert_eq!(
        who,
        [
            ("Sam Taylor", "contact"),
            ("Samira Okafor", "directory"),
            ("Alex Sampson", "directory")
        ]
    );
    assert_eq!(found["people"][1]["job_title"], "Engineer");
    assert_eq!(found["directory_searched"], true);

    // Offered while an address is typed, after anyone already written to.
    let offered = f
        .run("mail.people.suggest", json!({ "typed": "sam" }))
        .await;
    let addresses: Vec<&str> = offered["people"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["address"].as_str().unwrap())
        .collect();
    assert_eq!(
        addresses,
        [
            "sam@northwind.test",
            "samira.okafor@example.test",
            "alex.sampson@example.test"
        ]
    );

    // Out of reach: contacts are still found, and it is said the directory was not asked.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 200));
    let offline = f.run("people.search", json!({ "query": "sam" })).await;
    assert_eq!(offline["people"].as_array().unwrap().len(), 1);
    assert_eq!(offline["directory_searched"], false);
    assert!(
        f.registry
            .execute("contacts.delete", json!({ "contact_id": jo }))
            .await
            .is_err()
    );
    f.graph.clear_faults();

    f.run("contacts.delete", json!({ "contact_id": jo })).await;
    assert!(
        f.registry
            .execute("contacts.get", json!({ "contact_id": jo }))
            .await
            .is_err()
    );
    assert_eq!(f.graph.contacts(&suppliers).len(), 1);
}

#[tokio::test]
async fn a_shared_mailbox_is_added_by_address_and_read_changed_and_sent_from() {
    let f = Fixture::signed_in().await;
    f.graph.add_shared_mailbox("support@example.test", true);
    let (inbox, first) = f
        .graph
        .in_shared("support@example.test", |mailbox| {
            let inbox = mailbox.folder_id("inbox").unwrap();
            let first = mailbox.add_message(
                &inbox,
                message("Printer is down", "Alex Chen", "2026-10-02T09:00:00Z"),
            );
            mailbox.add_message(
                &inbox,
                message("Password reset", "Jo Nguyen", "2026-10-03T09:00:00Z"),
            );
            (inbox, first)
        })
        .unwrap();
    f.graph.add_message(
        &f.inbox,
        message("Mine", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.sync().await;

    // One there is no access to is not added, and says why; nor is what is not an address.
    let refused = f
        .registry
        .execute(
            "accounts.shared.add",
            json!({ "address": "payroll@example.test" }),
        )
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("not been given access"),
        "{refused}"
    );
    assert!(
        f.registry
            .execute("accounts.shared.add", json!({ "address": "support" }))
            .await
            .is_err()
    );
    assert_eq!(
        f.run("accounts.shared.list", json!({})).await["mailboxes"],
        json!([])
    );

    let added = f
        .run(
            "accounts.shared.add",
            json!({ "address": "Support@example.test" }),
        )
        .await["mailbox"]
        .clone();
    let id = added["id"].as_str().unwrap().to_owned();
    assert_eq!(added["address"], "support@example.test");
    assert!(
        f.registry
            .execute(
                "accounts.shared.add",
                json!({ "address": "support@example.test" })
            )
            .await
            .is_err()
    );

    // It syncs with the account, into a copy of its own.
    let synced = f.run("mail.sync", json!({})).await;
    assert_eq!(synced["accounts"].as_array().unwrap().len(), 2);
    assert_eq!(synced["accounts"][1]["account_id"], id);
    assert_eq!(synced["accounts"][1]["ok"], true, "{synced}");
    let subjects = |found: &Value| -> Vec<String> {
        found["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["subject"].as_str().unwrap().to_owned())
            .collect()
    };
    let theirs = f
        .run(
            "mail.messages.list",
            json!({ "account_id": id, "folder_id": inbox }),
        )
        .await;
    assert_eq!(subjects(&theirs), ["Password reset", "Printer is down"]);
    // The account's own mail is as it was, and none of the shared mailbox's is in it.
    let mine = f
        .run("mail.messages.list", json!({ "folder_id": f.inbox }))
        .await;
    assert_eq!(subjects(&mine), ["Mine"]);
    assert_eq!(
        subjects(
            &f.run(
                "mail.search",
                json!({ "account_id": id, "query": "printer" })
            )
            .await
        ),
        ["Printer is down"]
    );

    // A change made here goes to the shared mailbox, not the owner's.
    f.run(
        "mail.messages.set_read",
        json!({ "account_id": id, "message_ids": [first], "is_read": true }),
    )
    .await;
    f.run("mail.sync", json!({})).await;
    assert_eq!(
        f.graph
            .in_shared("support@example.test", |mailbox| mailbox
                .message(&first)
                .unwrap()["isRead"]
                .clone())
            .unwrap(),
        true
    );

    // A reply written in it is sent from it, and kept in its own Sent Items.
    let draft = f
        .run(
            "mail.drafts.create",
            json!({ "account_id": id, "kind": "reply", "message_id": first }),
        )
        .await["draft"]["id"]
        .clone();
    f.run(
        "mail.drafts.edit",
        json!({ "account_id": id, "draft_id": draft, "body": "On our way." }),
    )
    .await;
    f.run(
        "mail.drafts.send",
        json!({ "account_id": id, "draft_id": draft }),
    )
    .await;
    for _ in 0..20 {
        f.run("mail.sync", json!({})).await;
        if f.graph.sent().len() == 1 {
            break;
        }
    }
    assert_eq!(f.graph.sent().len(), 1);
    let in_sent = |mailbox: &mut brolga_fake_graph::FakeMailbox| {
        let sent = mailbox.folder_id("sentitems").unwrap();
        mailbox
            .messages()
            .values()
            .filter(|each| each.0 == sent)
            .count()
    };
    assert_eq!(f.graph.in_shared("support@example.test", in_sent), Some(1));

    // Removed: it is asked for no more, and its copy here is gone; the mailbox is untouched.
    assert_eq!(
        f.run("accounts.shared.remove", json!({ "mailbox_id": id }))
            .await["removed"],
        true
    );
    assert_eq!(
        f.run("mail.sync", json!({})).await["accounts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        f.registry
            .execute(
                "mail.messages.list",
                json!({ "account_id": id, "folder_id": inbox })
            )
            .await
            .is_err()
    );
    assert_eq!(f.graph.in_shared("support@example.test", in_sent), Some(1));
}

#[tokio::test]
async fn a_message_goes_out_under_a_shared_mailbox_s_name_only_with_leave_to() {
    let f = Fixture::signed_in().await;
    f.graph.add_shared_mailbox("support@example.test", true);
    f.graph.add_shared_mailbox("payroll@example.test", false);
    f.sync().await;
    let f = &f;
    let write = |from: &'static str| async move {
        let draft = f
            .run("mail.drafts.create", json!({ "to": [{ "address": "sam@example.test" }], "subject": "Hello", "body": "Hi." }))
            .await["draft"]["id"]
            .clone();
        let edited = f
            .run(
                "mail.drafts.edit",
                json!({ "draft_id": draft, "from": from }),
            )
            .await;
        assert_eq!(edited["draft"]["from"], from);
        f.run("mail.drafts.send", json!({ "draft_id": draft }))
            .await;
        for _ in 0..10 {
            f.sync().await;
        }
        draft
    };

    // With leave, it goes, under that name.
    write("support@example.test").await;
    let sent = f.graph.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].from_address, "support@example.test");

    // Without, it does not, and the server's own words say why; the draft is kept to put right.
    let refused = write("payroll@example.test").await;
    assert_eq!(f.graph.sent().len(), 1);
    let kept = f
        .run("mail.drafts.get", json!({ "draft_id": refused }))
        .await["draft"]
        .clone();
    assert_eq!(kept["state"], "failed");
    assert!(
        kept["error"]
            .as_str()
            .unwrap()
            .contains("does not have the right to send mail on behalf"),
        "{kept}"
    );

    // Put back to one's own name, it goes.
    f.run(
        "mail.drafts.edit",
        json!({ "draft_id": refused, "from": "" }),
    )
    .await;
    f.run("mail.drafts.send", json!({ "draft_id": refused }))
        .await;
    for _ in 0..10 {
        f.sync().await;
    }
    assert_eq!(f.graph.sent().len(), 2);
    assert_eq!(f.graph.sent()[1].from_address, "robin@example.test");
    assert!(
        f.registry
            .execute(
                "mail.drafts.edit",
                json!({ "draft_id": refused, "from": "not an address" })
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn the_mailboxes_categories_are_added_recoloured_and_deleted_on_the_server() {
    let f = Fixture::signed_in().await;

    let made = f
        .run(
            "mail.categories.create",
            json!({ "name": "Training", "color": "preset4" }),
        )
        .await;
    assert_eq!(
        made["categories"][3],
        json!({ "name": "Training", "color": "preset4" })
    );
    assert!(
        f.graph
            .master_categories()
            .contains(&("Training".to_owned(), "preset4".to_owned()))
    );

    // A second of the same name is refused before the server is asked for one.
    let again = f
        .registry
        .execute("mail.categories.create", json!({ "name": "training" }))
        .await
        .unwrap_err();
    assert!(again.to_string().contains("already"), "{again}");

    let changed = f
        .run(
            "mail.categories.set_color",
            json!({ "name": "Training", "color": "none" }),
        )
        .await;
    assert_eq!(changed["categories"][3]["color"], "none");
    // What was read is kept, for when the server cannot be asked.
    assert_eq!(f.run("mail.categories.list", json!({})).await, changed);

    let gone = f
        .run("mail.categories.delete", json!({ "name": "Training" }))
        .await;
    assert_eq!(gone["categories"].as_array().unwrap().len(), 3);
    assert_eq!(f.graph.master_categories().len(), 3);
    // Deleting what is not there is not a failure.
    f.run("mail.categories.delete", json!({ "name": "Training" }))
        .await;

    // Out of reach, nothing is changed and it says so.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 20));
    let error = f
        .registry
        .execute("mail.categories.create", json!({ "name": "Later" }))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("could not be reached"),
        "{error}"
    );
}

#[tokio::test]
async fn rules_are_listed_made_changed_and_deleted_on_the_server() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let archive = f.graph.add_folder("Receipts", None);
    f.sync().await;
    f.graph.add_rule(json!({
        "displayName": "Newsletters",
        "conditions": { "senderContains": ["newsletter"], "fromAddresses": [{ "emailAddress": { "address": "news@example.test", "name": "News" } }] },
        "exceptions": { "sentOnlyToMe": true },
        "actions": { "markAsRead": true, "stopProcessingRules": true }
    }));
    let locked = f.graph.add_rule(json!({
        "displayName": "Set by the organisation",
        "isReadOnly": true,
        "actions": { "delete": true }
    }));

    let listed = f.run("mail.rules.list", json!({})).await;
    let first = &listed["rules"][0];
    assert_eq!(first["name"], "Newsletters");
    assert_eq!(
        first["conditions"]["fromAddresses"],
        json!([{ "address": "news@example.test", "name": "News" }])
    );
    assert_eq!(
        first["summary"],
        "When it is from news@example.test and the sender contains “newsletter”, unless it is sent only to me: mark it read, stop there."
    );
    assert_eq!(listed["rules"][1]["read_only"], true);
    assert_eq!(
        listed["rules"][1]["summary"],
        "For every message: delete it."
    );

    // A folder is named by id or, for a standard one, by what it is.
    let made = f
        .run(
            "mail.rules.create",
            json!({
                "name": "Receipts",
                "conditions": { "subjectContains": ["receipt", "tax invoice"], "hasAttachments": true },
                "actions": { "moveToFolder": archive, "forwardTo": ["books@example.test"] }
            }),
        )
        .await;
    let id = made["rule"]["id"].as_str().unwrap().to_owned();
    assert_eq!(made["rule"]["sequence"], 3);
    assert_eq!(
        made["rule"]["summary"],
        "When the subject contains “receipt” or “tax invoice” and it has attachments: move it to Receipts, forward it to books@example.test."
    );
    let server = f.graph.rules();
    assert_eq!(server[2]["actions"]["moveToFolder"], archive);
    assert_eq!(
        server[2]["actions"]["forwardTo"][0]["emailAddress"]["address"],
        "books@example.test"
    );

    // Only what is given changes, and a part given is replaced whole.
    let changed = f
        .run(
            "mail.rules.update",
            json!({ "rule_id": id, "enabled": false, "actions": { "moveToFolder": "archive" } }),
        )
        .await;
    assert_eq!(changed["rule"]["enabled"], false);
    assert_eq!(changed["rule"]["name"], "Receipts");
    assert!(changed["rule"]["actions"].get("forwardTo").is_none());
    assert_eq!(
        f.graph.rules()[2]["conditions"]["hasAttachments"],
        json!(true)
    );

    // What cannot be a rule is refused here, before anything is sent.
    for (wrong, why) in [
        (json!({ "name": "x", "actions": {} }), "less than 1"),
        (
            json!({ "name": "x", "actions": { "moveToFolder": "no-such-folder" } }),
            "no folder",
        ),
        (
            json!({ "name": "x", "actions": { "forwardTo": ["not an address"] } }),
            "not an email address",
        ),
        (
            json!({ "name": "x", "actions": { "markAsRead": true }, "conditions": { "subjectHas": ["x"] } }),
            "subjectHas",
        ),
    ] {
        let error = f
            .registry
            .execute("mail.rules.create", wrong)
            .await
            .unwrap_err();
        assert!(error.to_string().contains(why), "{error}");
    }
    assert_eq!(f.graph.rules().len(), 3);

    // A read-only rule is seen and not touched.
    for command in ["mail.rules.update", "mail.rules.delete"] {
        let error = f
            .registry
            .execute(command, json!({ "rule_id": locked, "enabled": false }))
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("read-only") || error.to_string().contains("enabled"),
            "{error}"
        );
    }

    assert_eq!(
        f.run("mail.rules.delete", json!({ "rule_id": id })).await["deleted"],
        true
    );
    assert_eq!(f.graph.rules().len(), 2);

    // Out of reach, it says so and nothing is made.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 20));
    let error = f
        .registry
        .execute(
            "mail.rules.create",
            json!({ "name": "Later", "actions": { "markAsRead": true } }),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("could not be reached"),
        "{error}"
    );
}

#[tokio::test]
async fn a_rule_made_whose_answer_is_lost_is_found_and_not_made_twice() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    f.graph.lose_next_rule_answer();

    let made = f
        .run(
            "mail.rules.create",
            json!({ "name": "Once", "actions": { "markAsRead": true } }),
        )
        .await;

    assert_eq!(made["rule"]["name"], "Once");
    assert_eq!(f.graph.rules().len(), 1);
}

#[tokio::test]
async fn a_shared_mailbox_brings_its_calendar_which_is_read_and_written_in_that_mailbox() {
    let f = Fixture::signed_in().await;
    f.graph.add_shared_mailbox("support@example.test", true);
    f.graph.add_shared_event(
        "support@example.test",
        brolga_fake_graph::FakeEvent::new(
            "Roster handover",
            "2026-10-14T01:00:00Z",
            "2026-10-14T02:00:00Z",
        ),
    );
    f.sync().await;
    let added = f
        .run(
            "accounts.shared.add",
            json!({ "address": "support@example.test" }),
        )
        .await;
    let shared = added["mailbox"]["id"].as_str().unwrap().to_owned();
    f.run("mail.sync", json!({})).await;
    let week = json!({ "account_id": shared, "start": "2026-10-12T00:00:00Z", "end": "2026-10-19T00:00:00Z" });

    // Its events are its own, and not among the account's.
    let theirs = f.run("calendar.events.list", week.clone()).await;
    assert_eq!(theirs["events"].as_array().unwrap().len(), 1);
    assert_eq!(theirs["events"][0]["subject"], "Roster handover");
    let mine = f
        .run(
            "calendar.events.list",
            json!({ "start": "2026-10-12T00:00:00Z", "end": "2026-10-19T00:00:00Z" }),
        )
        .await;
    assert!(mine["events"].as_array().unwrap().is_empty());

    // An event made there is made in that mailbox's calendar, not one's own.
    let made = f
        .run(
            "calendar.events.create",
            json!({
                "account_id": shared,
                "subject": "Stocktake",
                "start": "2026-10-15T09:00:00",
                "end": "2026-10-15T10:00:00",
                "time_zone": "Australia/Brisbane"
            }),
        )
        .await;
    assert_eq!(made["event"]["subject"], "Stocktake");
    assert_eq!(
        f.graph
            .shared_events(
                "support@example.test",
                "2026-10-12T00:00:00Z",
                "2026-10-19T00:00:00Z"
            )
            .len(),
        2
    );
    let own = f.graph.default_calendar();
    assert!(
        f.graph
            .events(&own, "2026-10-12T00:00:00Z", "2026-10-19T00:00:00Z")
            .is_empty()
    );
    assert_eq!(
        f.run("calendar.events.list", week).await["events"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn a_calendar_someone_shared_without_their_mail_is_added_as_a_calendar_alone() {
    let f = Fixture::signed_in().await;
    f.graph.add_shared_calendar("alex@example.test");
    f.graph.add_shared_event(
        "alex@example.test",
        brolga_fake_graph::FakeEvent::new(
            "Board meeting",
            "2026-10-14T01:00:00Z",
            "2026-10-14T02:00:00Z",
        ),
    );
    f.sync().await;

    let added = f
        .run(
            "accounts.shared.add",
            json!({ "address": "alex@example.test" }),
        )
        .await;
    assert_eq!(added["mailbox"]["calendar_only"], true);
    let id = added["mailbox"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        f.run("accounts.shared.list", json!({})).await["mailboxes"][0]["calendar_only"],
        true
    );

    // It syncs without its mail being asked for, and without that being a failure.
    let synced = f.run("mail.sync", json!({})).await;
    let theirs = synced["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|each| each["account_id"] == id.as_str())
        .unwrap();
    assert_eq!(theirs["ok"], true, "{theirs}");
    assert_eq!(theirs["calendar_error"], Value::Null);
    assert!(
        !f.graph
            .requests()
            .iter()
            .any(|each| each.contains("alex@example.test/mailFolders/delta")),
    );

    let events = f
        .run(
            "calendar.events.list",
            json!({ "account_id": id, "start": "2026-10-12T00:00:00Z", "end": "2026-10-19T00:00:00Z" }),
        )
        .await;
    assert_eq!(events["events"][0]["subject"], "Board meeting");
}

#[tokio::test]
async fn contacts_are_read_from_a_vcard_file_and_written_to_one() {
    let f = Fixture::signed_in().await;
    f.graph.add_contact(
        None,
        json!({ "givenName": "Sam", "surname": "Taylor", "emailAddresses": [{ "address": "sam@example.test" }] }),
    );
    f.sync().await;
    let file = "BEGIN:VCARD\nVERSION:3.0\nFN:Sam Taylor\nN:Taylor;Sam;;;\nEMAIL:SAM@example.test\nEND:VCARD\n\
                BEGIN:VCARD\nVERSION:2.1\nN:Chen;Alex\nFN:Alex Chen\nORG:Northwind;Accounts\nTEL;CELL:0411 111 111\n\
                EMAIL;INTERNET:alex@example.test\nBDAY:1984-03-09\nCATEGORIES:Clients\nEND:VCARD\n\
                BEGIN:VCARD\nVERSION:3.0\nTEL:123\nEND:VCARD\n\
                BEGIN:VCARD\nVERSION:4.0\nFN:Jo Nguyen\nEMAIL:jo@example.test\nEND:VCARD\n";

    let read = f.run("contacts.import", json!({ "text": file })).await;

    assert_eq!(read["imported"], 2);
    // One already here by its address, whatever its case, and one that names nobody.
    assert_eq!(read["already_here"], 1);
    assert_eq!(read["skipped"], 1);
    assert_eq!(read["stopped"], Value::Null);
    let server = f.graph.contacts("");
    assert_eq!(server.len(), 3);
    let alex = server
        .iter()
        .find(|each| each["displayName"] == "Alex Chen")
        .unwrap();
    assert_eq!(alex["companyName"], "Northwind");
    assert_eq!(alex["department"], "Accounts");
    assert_eq!(alex["mobilePhone"], "0411 111 111");
    assert_eq!(alex["birthday"], "1984-03-09T11:59:00Z");
    assert_eq!(alex["categories"], json!(["Clients"]));
    // They are here at once, without a sync.
    let listed = f.run("contacts.list", json!({ "query": "alex" })).await;
    assert_eq!(listed["contacts"][0]["birthday"], "1984-03-09");

    // Read again, nobody is added twice.
    let again = f.run("contacts.import", json!({ "text": file })).await;
    assert_eq!(again["imported"], 0);
    assert_eq!(again["already_here"], 3);
    assert_eq!(f.graph.contacts("").len(), 3);

    // Written out, and read back by its path, they are all already here.
    let written = f.run("contacts.export", json!({})).await;
    assert_eq!(written["count"], 3);
    let path = written["path"].as_str().unwrap();
    assert!(path.ends_with("contacts.vcf"), "{path}");
    let text = std::fs::read_to_string(path).unwrap();
    assert_eq!(text.matches("BEGIN:VCARD").count(), 3);
    assert!(text.contains("ORG:Northwind;Accounts"), "{text}");
    let back = f.run("contacts.import", json!({ "path": path })).await;
    assert_eq!(back["already_here"], 3);
    // A second file does not write over the first.
    let second = f.run("contacts.export", json!({})).await;
    assert_ne!(second["path"], written["path"]);

    // What is not a file of cards says so, and adds nobody.
    for wrong in [
        json!({ "text": "Dear Sam, here are my details." }),
        json!({ "path": "relative.vcf" }),
        json!({ "path": "/no/such/file.vcf" }),
        json!({}),
    ] {
        assert!(f.registry.execute("contacts.import", wrong).await.is_err());
    }

    // Out of reach part of the way, those made are kept and it says where it stopped.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 20));
    let stopped = f
        .run(
            "contacts.import",
            json!({ "text": "BEGIN:VCARD\nVERSION:3.0\nFN:Later\nEMAIL:later@example.test\nEND:VCARD\n" }),
        )
        .await;
    assert_eq!(stopped["imported"], 0);
    assert!(
        stopped["stopped"]
            .as_str()
            .unwrap()
            .contains("stopped answering")
    );
}

#[tokio::test]
async fn a_persons_card_says_who_they_are_their_last_mail_and_the_meetings_to_come() {
    let f = Fixture::signed_in().await;
    f.graph.add_person(
        "Sam Taylor",
        "sam.taylor@example.test",
        "Buyer",
        "Purchasing",
    );
    for n in 1..=7 {
        f.graph.add_message(
            &f.inbox,
            message(
                &format!("Order {n}"),
                "Sam Taylor",
                &format!("2026-10-0{n}T09:00:00Z"),
            ),
        );
    }
    f.graph.add_message(
        &f.inbox,
        message("From someone else", "Alex Chen", "2026-10-08T09:00:00Z"),
    );
    let main = f.graph.default_calendar();
    let soon = |days: i64| {
        jiff::Timestamp::now()
            .checked_add(jiff::SignedDuration::from_hours(24 * days))
            .unwrap()
            .strftime("%Y-%m-%dT%H:00:00Z")
            .to_string()
    };
    f.graph.add_event(
        &main,
        brolga_fake_graph::FakeEvent::new("Supplier review", &soon(3), &soon(4))
            .invited_by("Sam Taylor", "Sam.Taylor@example.test"),
    );
    f.graph.add_event(
        &main,
        brolga_fake_graph::FakeEvent::new("Nothing to do with Sam", &soon(5), &soon(6)),
    );
    f.sync().await;

    let card = f
        .run(
            "people.card",
            json!({ "address": " Sam.Taylor@Example.test " }),
        )
        .await;

    assert_eq!(card["address"], "sam.taylor@example.test");
    assert_eq!(card["person"]["name"], "Sam Taylor");
    assert_eq!(card["person"]["job_title"], "Buyer");
    let mail = card["recent_mail"].as_array().unwrap();
    assert_eq!(mail.len(), 5);
    assert_eq!(mail[0]["subject"], "Order 7");
    let meetings = card["upcoming_meetings"].as_array().unwrap();
    assert_eq!(meetings.len(), 1);
    assert_eq!(meetings[0]["subject"], "Supplier review");

    // Someone nobody knows is still a card, with nothing on it; and with the
    // server out of reach the mail and meetings are still there.
    let stranger = f
        .run("people.card", json!({ "address": "nobody@elsewhere.test" }))
        .await;
    assert_eq!(stranger["person"], Value::Null);
    assert!(stranger["recent_mail"].as_array().unwrap().is_empty());
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 20));
    let offline = f
        .run(
            "people.card",
            json!({ "address": "sam.taylor@example.test" }),
        )
        .await;
    assert_eq!(offline["recent_mail"].as_array().unwrap().len(), 5);
    assert_eq!(offline["upcoming_meetings"].as_array().unwrap().len(), 1);

    assert!(
        f.registry
            .execute("people.card", json!({ "address": "not an address" }))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn the_main_calendar_is_shared_changed_and_unshared_on_the_server() {
    let f = Fixture::signed_in().await;

    // At first, only the organisation as a whole, which cannot be removed.
    let listed = f.run("calendar.sharing.list", json!({})).await;
    let whole = &listed["shares"][0];
    assert_eq!(whole["address"], Value::Null);
    assert_eq!(whole["role"], "freeBusyRead");
    assert_eq!(whole["removable"], false);

    let made = f
        .run(
            "calendar.sharing.set",
            json!({ "address": " Sam@Example.test ", "name": "Sam Taylor", "role": "read" }),
        )
        .await;
    assert_eq!(made["share"]["address"], "sam@example.test");
    assert_eq!(made["share"]["inside_organization"], true);
    let id = made["share"]["id"].as_str().unwrap().to_owned();

    // Named again by address, the same share is changed and no second one made.
    let changed = f
        .run(
            "calendar.sharing.set",
            json!({ "address": "sam@example.test", "role": "write" }),
        )
        .await;
    assert_eq!(changed["share"]["id"], id.as_str());
    assert_eq!(
        f.graph.calendar_shares(),
        [
            (String::new(), "freeBusyRead".to_owned()),
            ("sam@example.test".to_owned(), "write".to_owned())
        ]
    );

    // The organisation is named by its share, and may be shown nothing.
    f.run(
        "calendar.sharing.set",
        json!({ "share_id": whole["id"], "role": "none" }),
    )
    .await;
    assert_eq!(f.graph.calendar_shares()[0].1, "none");

    // What Microsoft 365 will not allow is said, with what it will.
    let outsider = f
        .run(
            "calendar.sharing.set",
            json!({ "address": "pat@elsewhere.test", "role": "read" }),
        )
        .await;
    assert_eq!(outsider["share"]["inside_organization"], false);
    let refused = f
        .registry
        .execute(
            "calendar.sharing.set",
            json!({ "address": "pat@elsewhere.test", "role": "write" }),
        )
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("freeBusyRead, limitedRead, read"),
        "{refused}"
    );
    for wrong in [
        json!({ "role": "read" }),
        json!({ "address": "not an address", "role": "read" }),
        json!({ "share_id": "no-such", "role": "read" }),
        json!({ "address": "new@example.test", "role": "none" }),
    ] {
        assert!(
            f.registry
                .execute("calendar.sharing.set", wrong)
                .await
                .is_err()
        );
    }

    assert_eq!(
        f.run("calendar.sharing.remove", json!({ "share_id": id }))
            .await["removed"],
        true
    );
    assert_eq!(f.graph.calendar_shares().len(), 2);
    assert!(
        f.registry
            .execute(
                "calendar.sharing.remove",
                json!({ "share_id": whole["id"] })
            )
            .await
            .is_err()
    );

    // Out of reach, it says so.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 20));
    let error = f
        .registry
        .execute("calendar.sharing.list", json!({}))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("could not be reached"),
        "{error}"
    );
}

/// A PST file of this test's own: two messages in the inbox, one with a
/// picture within it and a file attached, and one in a folder two deep.
fn sample_pst(name: &str) -> std::path::PathBuf {
    use pst_writer::{
        UnicodePstAttachment, UnicodePstFile, UnicodePstMessage, UnicodePstRecipient,
        UnicodePstRecipientType,
    };
    let folder = downloads().join(name);
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("Old mail.pst");
    let recipients = [
        UnicodePstRecipient {
            name: "Robin Example",
            email: "robin@example.test",
            recipient_type: UnicodePstRecipientType::To,
        },
        UnicodePstRecipient {
            name: "Jo Nguyen",
            email: "jo@example.test",
            recipient_type: UnicodePstRecipientType::Cc,
        },
    ];
    let first = UnicodePstMessage {
        subject: "Harbour tender",
        sender_name: "Sam Taylor",
        sender_email: "sam@example.test",
        recipients: &recipients,
        body: "The tender closes on Friday.",
        html_body: Some(
            "<p>The tender closes on <b>Friday</b>.</p><img src=\"cid:plan@example.test\"><script>alert(1)</script>",
        ),
        message_id: "<one@example.test>",
        delivery_time: 133_750_656_000_000_000,
    };
    let attachments = [
        UnicodePstAttachment {
            filename: "plan.png",
            mime_type: "image/png",
            content_id: Some("<plan@example.test>"),
            data: &[0x89, 0x50, 0x4e, 0x47, 1, 2, 3],
        },
        UnicodePstAttachment {
            filename: "../tender.txt",
            mime_type: "text/plain",
            content_id: None,
            data: b"Terms of the tender.",
        },
    ];
    drop(UnicodePstFile::create_with_attachments(&path, &first, &attachments).unwrap());
    drop(
        UnicodePstFile::append(
            &path,
            &UnicodePstMessage {
                subject: "Lunch",
                body: "Noodles on Tuesday?",
                html_body: None,
                message_id: "<two@example.test>",
                ..first
            },
        )
        .unwrap(),
    );
    drop(
        UnicodePstFile::append_in_folder(
            &path,
            &["Projects", "Wharf"],
            &UnicodePstMessage {
                subject: "Piling survey",
                body: "The survey is attached.",
                html_body: None,
                message_id: "<three@example.test>",
                ..first
            },
        )
        .unwrap(),
    );

    path
}

/// Wait for an import to stop, and give what the list then says of it.
async fn imported(f: &Fixture, id: &str) -> Value {
    for _ in 0..400 {
        let listed = f.run("archives.list", json!({})).await;
        let archive = listed["archives"]
            .as_array()
            .unwrap()
            .iter()
            .find(|each| each["id"] == id)
            .unwrap()
            .clone();
        if archive["state"] != "importing" {
            return archive;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the import never finished");
}

#[tokio::test]
async fn a_pst_file_is_read_into_an_archive_that_is_read_and_searched_like_a_mailbox() {
    let f = Fixture::signed_in().await;
    f.sync().await;
    let path = sample_pst("whole");
    let before = std::fs::read(&path).unwrap();

    let started = f
        .run("archives.import", json!({ "path": path.to_string_lossy() }))
        .await;
    let id = started["archive"]["id"].as_str().unwrap().to_owned();
    assert_eq!(started["archive"]["name"], "Old mail");
    assert_eq!(started["archive"]["state"], "importing");
    let done = imported(&f, &id).await;
    assert_eq!(done["state"], "ready", "{done}");
    assert_eq!(done["messages"], 3);
    assert_eq!(done["skipped"], 0);
    // The file is exactly as it was.
    assert_eq!(std::fs::read(&path).unwrap(), before);

    // Its folders are a tree, with the inbox known for what it is.
    let folders = f
        .run("mail.folders.list", json!({ "account_id": id }))
        .await;
    let folder = |name: &str| {
        folders["folders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|each| each["name"] == name)
            .unwrap_or_else(|| panic!("no folder {name} in {folders}"))
            .clone()
    };
    assert_eq!(folder("Inbox")["well_known"], "inbox");
    assert_eq!(folder("Inbox")["total"], 2);
    assert_eq!(folder("Wharf")["parent_id"], folder("Projects")["id"]);
    assert_eq!(folder("Wharf")["depth"], 1);

    let listed = f
        .run(
            "mail.messages.list",
            json!({ "account_id": id, "folder_id": folder("Inbox")["id"] }),
        )
        .await;
    let tender = listed["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|each| each["subject"] == "Harbour tender")
        .unwrap_or_else(|| panic!("{listed}"));
    assert_eq!(tender["from"]["address"], "sam@example.test");
    assert_eq!(tender["received_at"], "2024-11-03T00:00:00Z");
    assert_eq!(tender["has_attachments"], true);
    let message = tender["id"].as_str().unwrap();

    // Its body is here, made safe like any other, with its picture shown from the archive.
    let opened = f
        .run(
            "mail.messages.get",
            json!({ "account_id": id, "message_id": message, "format": "html" }),
        )
        .await;
    let body = opened["body"]["content"].as_str().unwrap();
    assert!(body.contains("<b>Friday</b>"), "{opened}");
    assert!(!body.contains("script"), "{body}");
    assert!(body.contains("data:image/png;base64,"), "{body}");
    assert_eq!(opened["message"]["to"][0]["address"], "robin@example.test");
    assert_eq!(opened["message"]["cc"][0]["address"], "jo@example.test");

    // Its attachment is saved from the archive, under a name made safe.
    let attached = f
        .run(
            "mail.attachments.list",
            json!({ "account_id": id, "message_id": message }),
        )
        .await;
    let file = attached["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|each| each["is_inline"] == false)
        .unwrap();
    let saved = f
        .run(
            "mail.attachments.save",
            json!({ "account_id": id, "message_id": message, "attachment_id": file["id"] }),
        )
        .await;
    let saved = std::path::PathBuf::from(saved["path"].as_str().unwrap());
    assert_eq!(std::fs::read(&saved).unwrap(), b"Terms of the tender.");
    assert!(
        saved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("tender.txt")
    );
    assert!(!saved.to_string_lossy().contains(".."));

    // It is searched like any mailbox, and its mail is not among the account's own.
    let found = f
        .run(
            "mail.search",
            json!({ "account_id": id, "query": "survey" }),
        )
        .await;
    assert_eq!(found["messages"][0]["subject"], "Piling survey");
    let mine = f.run("mail.search", json!({ "query": "survey" })).await;
    assert!(mine["messages"].as_array().unwrap().is_empty());

    // A change is made here and is the whole of it: nothing waits to be sent.
    let requests = f.graph.requests().len();
    f.run(
        "mail.messages.set_read",
        json!({ "account_id": id, "message_ids": [message], "is_read": true }),
    )
    .await;
    let reread = f
        .run(
            "mail.messages.list",
            json!({ "account_id": id, "folder_id": folder("Inbox")["id"] }),
        )
        .await;
    assert!(
        reread["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|each| each["id"] == message && each["is_read"] == true)
    );
    let synced = f.run("mail.sync", json!({ "account_id": id })).await;
    assert_eq!(synced["accounts"][0]["ok"], true);
    assert_eq!(f.graph.requests().len(), requests);

    // What needs the server says there is none.
    let refused = f
        .registry
        .execute("calendar.sharing.list", json!({ "account_id": id }))
        .await
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("not connected to Microsoft 365"),
        "{refused}"
    );

    // Removed, it is gone and the file is still there, as it was.
    assert_eq!(
        f.run("archives.remove", json!({ "archive_id": id })).await["removed"],
        true
    );
    assert!(
        f.run("archives.list", json!({})).await["archives"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        f.registry
            .execute("mail.folders.list", json!({ "account_id": id }))
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[tokio::test]
async fn a_file_that_is_not_a_pst_fails_and_says_so_and_a_missing_one_makes_no_archive() {
    let f = Fixture::signed_in().await;
    let folder = downloads().join("not-pst");
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("letter.pst");
    std::fs::write(&path, b"Dear Sam, this is not a mailbox.").unwrap();

    let started = f
        .run(
            "archives.import",
            json!({ "path": path.to_string_lossy(), "name": "  A letter " }),
        )
        .await;
    assert_eq!(started["archive"]["name"], "A letter");
    let done = imported(&f, started["archive"]["id"].as_str().unwrap()).await;
    assert_eq!(done["state"], "failed");
    assert!(
        done["error"].as_str().unwrap().contains("not a PST"),
        "{done}"
    );

    for wrong in [
        json!({ "path": "relative.pst" }),
        json!({ "path": "/no/such/file.pst" }),
    ] {
        assert!(f.registry.execute("archives.import", wrong).await.is_err());
    }
    assert_eq!(
        f.run("archives.list", json!({})).await["archives"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn an_import_that_is_stopped_keeps_what_it_had_read_and_no_more() {
    use pst_writer::{
        UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile, UnicodePstMessage,
    };
    let f = Fixture::signed_in().await;
    let folder = downloads().join("stopped");
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("large.pst");
    let first = UnicodePstMessage {
        subject: "One of many",
        sender_name: "Sam Taylor",
        sender_email: "sam@example.test",
        recipients: &[],
        body: "A message with a large file attached.",
        html_body: None,
        message_id: "<first@example.test>",
        delivery_time: 133_750_656_000_000_000,
    };
    drop(UnicodePstFile::create(&path, &first).unwrap());
    // Large enough that reading it all takes far longer than asking it to stop.
    let data = vec![0x5Au8; 512 * 1024];
    let attachments = [UnicodePstAttachment {
        filename: "large.bin",
        mime_type: "application/octet-stream",
        content_id: None,
        data: &data,
    }];
    let messages = vec![
        UnicodePstBatchMessage {
            message: first,
            attachments: &attachments
        };
        400
    ];
    drop(UnicodePstFile::append_many_with_attachments(&path, &messages).unwrap());

    let started = f
        .run("archives.import", json!({ "path": path.to_string_lossy() }))
        .await;
    let id = started["archive"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        f.run("archives.cancel", json!({ "archive_id": id })).await["cancelled"],
        true
    );
    let done = imported(&f, &id).await;

    assert_eq!(done["state"], "cancelled", "{done}");
    let kept = done["messages"].as_u64().unwrap();
    assert!(kept < 401, "{done}");
    // What was read is whole and can be read; what was not is simply not here.
    let folders = f
        .run("mail.folders.list", json!({ "account_id": id }))
        .await;
    let total: u64 = folders["folders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|each| each["total"].as_u64().unwrap())
        .sum();
    assert_eq!(total, kept);
    // Stopping one that has stopped is nothing.
    assert_eq!(
        f.run("archives.cancel", json!({ "archive_id": id })).await["cancelled"],
        false
    );
}

/// Not run with the rest: reads the PST named by `BROLGA_BIG_PST` into a
/// database under `BROLGA_BIG_DATA`, and says how long it took and the most
/// memory it held. `cargo test --release -p brolga-core --test mail a_large_pst -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "needs a large PST named by BROLGA_BIG_PST"]
async fn a_large_pst_imports_within_bounded_memory_and_is_searchable() {
    let path = std::env::var("BROLGA_BIG_PST").expect("BROLGA_BIG_PST");
    let data = std::path::PathBuf::from(std::env::var("BROLGA_BIG_DATA").expect("BROLGA_BIG_DATA"));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    let identity = FakeIdentity::start().await.unwrap();
    let core = Core::open(CoreOptions {
        data_dir: Some(data),
        downloads_dir: None,
        secrets: Arc::new(MemorySecrets::default()),
        open_browser: Arc::new(|_| Ok(())),
        authority: Authority::new(identity.base_url()),
        graph_base: None,
        default_client_id: None,
        fetch_images_from_private_hosts: false,
    })
    .unwrap();
    let registry = core.registry();
    let memory = || {
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status
                    .lines()
                    .find(|line| line.starts_with("VmHWM"))
                    .map(|line| line.split_whitespace().nth(1).unwrap_or("?").to_owned())
            })
            .unwrap_or_else(|| "?".to_owned())
    };

    let began = std::time::Instant::now();
    let started = registry
        .execute("archives.import", json!({ "path": path }))
        .await
        .unwrap();
    let id = started["archive"]["id"].as_str().unwrap().to_owned();
    let mut said = 0;
    let done = loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let listed = registry.execute("archives.list", json!({})).await.unwrap();
        let archive = listed["archives"][0].clone();
        if archive["state"] != "importing" {
            break archive;
        }
        let dealt = archive["messages_done"].as_u64().unwrap_or(0);
        if dealt >= said + 5000 {
            said = dealt;
            println!(
                "{dealt} of {} after {:?}, most memory {} kB",
                archive["messages_total"],
                began.elapsed(),
                memory()
            );
        }
    };
    println!("{done}");
    println!("took {:?}, most memory {} kB", began.elapsed(), memory());
    assert_eq!(done["state"], "ready");

    let searching = std::time::Instant::now();
    let found = registry
        .execute(
            "mail.search",
            json!({ "account_id": id, "query": "harbour survey", "limit": 5 }),
        )
        .await
        .unwrap();
    println!(
        "search took {:?} and found {}",
        searching.elapsed(),
        found["messages"].as_array().unwrap().len()
    );
    assert!(!found["messages"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_sync_marks_what_was_answered_or_forwarded_elsewhere() {
    let f = Fixture::signed_in().await;
    let quote = f.graph.add_message(
        &f.inbox,
        message("Quote", "Sam Taylor", "2026-10-01T09:00:00Z"),
    );
    f.graph.add_message(
        &f.inbox,
        message("Plan", "Sam Taylor", "2026-10-01T08:00:00Z"),
    );
    f.graph.set_last_verb(&quote, 103);

    f.sync().await;

    let listed = f
        .run("mail.messages.list", json!({ "folder_id": f.inbox }))
        .await;
    assert_eq!(
        (
            listed["messages"][0]["last_action"].clone(),
            listed["messages"][1]["last_action"].clone()
        ),
        (json!("replied_all"), Value::Null)
    );
}
