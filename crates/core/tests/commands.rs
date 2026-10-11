#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use brolga_auth::{Authority, MemorySecrets};
use brolga_core::{Core, CoreOptions};
use brolga_fake_graph::{FakeGraph, FakeIdentity, FakeMessage};
use serde_json::json;

const CLIENT_ID: &str = "11111111-2222-4333-8444-555555555555";

/// A folder of this test run's own to save attachments into.
fn downloads() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("brolga-commands-{}", std::process::id()))
}

/// A core signed in to nothing, pointed at the fake identity platform, whose
/// "browser" opens the page and follows it back as a person's would.
fn core(identity: &FakeIdentity, default_client_id: Option<&str>) -> Core {
    core_with_mail(identity, None, default_client_id)
}

fn core_with_mail(
    identity: &FakeIdentity,
    graph: Option<&FakeGraph>,
    default_client_id: Option<&str>,
) -> Core {
    Core::open(CoreOptions {
        data_dir: None,
        downloads_dir: Some(downloads()),
        secrets: Arc::new(MemorySecrets::default()),
        open_browser: Arc::new(|url| {
            // The example link is not the sign-in page, and goes nowhere.
            if !url.contains("/oauth2/") {
                return Ok(());
            }
            let url = url.to_owned();
            tokio::spawn(async move {
                let _ = reqwest::get(url).await;
            });
            Ok(())
        }),
        authority: Authority::new(identity.base_url()),
        graph_base: graph.map(FakeGraph::base_url),
        default_client_id: default_client_id.map(str::to_owned),
        fetch_images_from_private_hosts: false,
    })
    .unwrap()
}

/// The gate the build spec asks for: no command gets in without a
/// description for agents, a schema and an example, and every example runs.
#[tokio::test]
async fn every_command_is_described_and_its_examples_run() {
    let identity = FakeIdentity::start().await.unwrap();
    let graph = FakeGraph::start().await.unwrap();
    graph.add_message(
        &graph.folder_id("inbox"),
        FakeMessage::new(
            "Hello",
            "Sam Taylor",
            "sam@example.test",
            "2026-10-01T09:00:00Z",
        )
        .attach("notes.txt", "text/plain", b"Some notes."),
    );
    // A mailbox someone shared, for `accounts.shared.add`.
    graph.add_shared_mailbox("support@example.test", true);
    // A contact, for `contacts.get` and `contacts.update`.
    assert_eq!(
        graph.add_contact(None, serde_json::json!({ "givenName": "Sam", "surname": "Taylor", "emailAddresses": [{ "address": "sam@example.test" }] })),
        "contact-0001"
    );
    // An event, for `calendar.events.get`.
    assert_eq!(
        graph.add_event(
            &graph.default_calendar(),
            brolga_fake_graph::FakeEvent::new(
                "Dentist",
                "2026-10-20T01:00:00Z",
                "2026-10-20T02:00:00Z"
            ),
        ),
        "event-0001"
    );
    // A meeting someone else called, for `calendar.events.respond`.
    assert_eq!(
        graph.add_event(
            &graph.default_calendar(),
            brolga_fake_graph::FakeEvent::new(
                "Budget review",
                "2026-10-21T01:00:00Z",
                "2026-10-21T02:00:00Z"
            )
            .invited_by("Sam Taylor", "sam@example.test"),
        ),
        "event-0002"
    );
    // A plain event, so that the next has the id the examples expect; and
    // a meeting of one's own, for `calendar.events.cancel`.
    graph.add_event(
        &graph.default_calendar(),
        brolga_fake_graph::FakeEvent::new("Lunch", "2026-10-22T01:00:00Z", "2026-10-22T02:00:00Z"),
    );
    assert_eq!(
        graph.add_event(
            &graph.default_calendar(),
            brolga_fake_graph::FakeEvent::new(
                "Team meeting",
                "2026-10-23T01:00:00Z",
                "2026-10-23T02:00:00Z"
            ),
        ),
        "event-0004"
    );
    // A draft written in some other mail program, for `mail.drafts.open`.
    let mut elsewhere = FakeMessage::new(
        "Half written",
        "Robin Example",
        "robin@example.test",
        "2026-10-02T09:00:00Z",
    );
    elsewhere.is_draft = true;
    assert_eq!(
        graph.add_message(&graph.folder_id("drafts"), elsewhere),
        "message-000009"
    );
    let registry = core_with_mail(&identity, Some(&graph), Some(CLIENT_ID)).registry();
    // The mail examples need an account with mail in it, which no example
    // can arrange for itself.
    registry
        .execute("accounts.sign_in", json!({}))
        .await
        .unwrap();
    registry.execute("mail.sync", json!({})).await.unwrap();
    let mut seen = 0;

    for spec in registry.specs() {
        seen += 1;
        assert!(!spec.title.trim().is_empty(), "`{}` has no title", spec.id);
        assert!(
            spec.description.len() >= 20,
            "`{}` is not described for an agent",
            spec.id
        );
        assert_eq!(
            spec.args_schema["type"], "object",
            "`{}` must take an object",
            spec.id
        );
        assert!(
            !spec.examples.is_empty(),
            "`{}` has no example, so nothing tests it",
            spec.id
        );

        for example in &spec.examples {
            let result = registry.execute(&spec.id, example.clone()).await;
            assert!(
                result.is_ok(),
                "`{}` failed on its own example {example}: {result:?}",
                spec.id
            );
        }
    }

    assert!(seen >= 13, "commands have gone missing: {seen}");
}

#[tokio::test]
async fn signing_in_adds_the_account_and_removing_takes_it_away() {
    let identity = FakeIdentity::start().await.unwrap();
    let registry = core(&identity, Some(CLIENT_ID)).registry();

    let added = registry
        .execute("accounts.sign_in", json!({}))
        .await
        .unwrap();
    let listed = registry.execute("accounts.list", json!({})).await.unwrap();

    assert_eq!(added["account"]["username"], "robin@example.test");
    assert_eq!(added["account"]["needs_sign_in"], false);
    assert_eq!(listed["accounts"], json!([added["account"]]));

    let id = added["account"]["id"].as_str().unwrap();
    assert_eq!(
        registry
            .execute("accounts.remove", json!({ "id": id }))
            .await
            .unwrap(),
        json!({ "removed": true })
    );
    assert_eq!(
        registry.execute("accounts.list", json!({})).await.unwrap(),
        json!({ "accounts": [] })
    );
}

#[tokio::test]
async fn an_organisations_own_client_id_is_used_when_given() {
    let identity = FakeIdentity::start().await.unwrap();
    let registry = core(&identity, None).registry();

    registry
        .execute(
            "accounts.sign_in",
            json!({ "client_id": "own-client", "tenant": "contoso.example" }),
        )
        .await
        .unwrap();

    assert_eq!(identity.authorize_requests()[0]["client_id"], "own-client");
}

#[tokio::test]
async fn with_no_client_id_at_all_the_person_is_told_what_to_do() {
    let identity = FakeIdentity::start().await.unwrap();
    let registry = core(&identity, None).registry();

    let error = registry
        .execute("accounts.sign_in", json!({}))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("client ID"), "{error}");
    assert!(identity.authorize_requests().is_empty());
}

#[tokio::test]
async fn a_refused_sign_in_comes_back_in_plain_words() {
    let identity = FakeIdentity::start().await.unwrap();
    let registry = core(&identity, Some(CLIENT_ID)).registry();
    identity.refuse_next_token(
        "invalid_grant",
        &[90094],
        "AADSTS90094: Administrator consent is required.",
    );

    let error = registry
        .execute("accounts.sign_in", json!({}))
        .await
        .unwrap_err();

    assert!(
        error.to_string().contains("administrator to approve"),
        "{error}"
    );
    assert!(!error.to_string().contains("AADSTS"), "{error}");
    assert_eq!(
        registry.execute("accounts.list", json!({})).await.unwrap(),
        json!({ "accounts": [] })
    );
}

#[tokio::test]
async fn signing_in_with_a_code_shows_the_code_then_adds_the_account() {
    let identity = FakeIdentity::start().await.unwrap();
    let registry = core(&identity, Some(CLIENT_ID)).registry();

    let begun = registry
        .execute("accounts.sign_in_with_code.begin", json!({}))
        .await
        .unwrap();
    assert_eq!(begun["user_code"], "ABCD1234");
    let finished = registry
        .execute(
            "accounts.sign_in_with_code.finish",
            json!({ "flow_id": begun["flow_id"] }),
        )
        .await
        .unwrap();

    assert_eq!(finished["account"]["username"], "robin@example.test");
    // A flow is good once.
    assert!(
        registry
            .execute(
                "accounts.sign_in_with_code.finish",
                json!({ "flow_id": begun["flow_id"] })
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn an_account_that_needs_it_signs_in_again_as_it_was_added() {
    let identity = FakeIdentity::start().await.unwrap();
    let registry = core(&identity, None).registry();
    let added = registry
        .execute(
            "accounts.sign_in",
            json!({ "client_id": "own-client", "tenant": "contoso.example" }),
        )
        .await
        .unwrap();
    let id = added["account"]["id"].as_str().unwrap();

    let renewed = registry
        .execute("accounts.sign_in", json!({ "account_id": id }))
        .await
        .unwrap();

    assert_eq!(renewed["account"]["id"], id);
    let asked = &identity.authorize_requests()[1];
    assert_eq!(asked["client_id"], "own-client");
    assert_eq!(asked["login_hint"], "robin@example.test");
    assert!(
        registry
            .execute("accounts.sign_in", json!({ "account_id": "nobody" }))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn the_window_can_ask_whether_a_client_id_is_needed() {
    let identity = FakeIdentity::start().await.unwrap();

    let without = core(&identity, None)
        .registry()
        .execute("accounts.sign_in_options", json!({}))
        .await
        .unwrap();
    let with = core(&identity, Some(CLIENT_ID))
        .registry()
        .execute("accounts.sign_in_options", json!({}))
        .await
        .unwrap();

    assert_eq!(without["needs_client_id"], true);
    assert_eq!(with["needs_client_id"], false);
}

#[tokio::test]
async fn the_client_id_is_typed_once_and_remembered_for_the_next_account() {
    let identity = FakeIdentity::start().await.unwrap();
    let registry = core(&identity, None).registry();
    let options = || async {
        registry
            .execute("accounts.sign_in_options", json!({}))
            .await
            .unwrap()
    };

    assert_eq!(options().await["needs_client_id"], true);
    assert!(
        registry
            .execute("accounts.sign_in", json!({}))
            .await
            .is_err()
    );

    registry
        .execute("accounts.sign_in", json!({ "client_id": CLIENT_ID }))
        .await
        .unwrap();

    assert_eq!(
        options().await,
        json!({ "needs_client_id": false, "remembered_client_id": CLIENT_ID })
    );
    // The next sign-in needs none, and goes out under the same one.
    registry
        .execute("accounts.sign_in", json!({}))
        .await
        .unwrap();
    assert_eq!(
        identity.authorize_requests().last().unwrap()["client_id"],
        CLIENT_ID
    );

    // It outlasts the account it came with.
    let id =
        registry.execute("accounts.list", json!({})).await.unwrap()["accounts"][0]["id"].clone();
    registry
        .execute("accounts.remove", json!({ "id": id }))
        .await
        .unwrap();
    assert_eq!(options().await["needs_client_id"], false);
}

#[tokio::test]
async fn the_commands_an_agent_may_draft_with_are_real_and_are_all_drafting() {
    let identity = FakeIdentity::start().await.unwrap();
    let registry = core(&identity, Some(CLIENT_ID)).registry();

    for id in brolga_commands::drafting_commands() {
        let spec = registry
            .spec(id)
            .unwrap_or_else(|| panic!("`{id}` is not a command"));
        assert_eq!(spec.access(), brolga_commands::Access::Draft);
        assert!(!spec.consequential, "`{id}` reaches other people");
    }
    // What reaches other people is never let through as drafting or reading.
    for spec in registry.specs().filter(|s| s.consequential) {
        assert_eq!(
            spec.access(),
            brolga_commands::Access::Full,
            "`{}`",
            spec.id
        );
    }
    assert_eq!(
        registry.spec("mail.sync").unwrap().access(),
        brolga_commands::Access::Read
    );
}

#[tokio::test]
async fn a_question_for_the_user_waits_for_the_window_and_takes_its_answer() {
    let identity = FakeIdentity::start().await.unwrap();
    let registry = core(&identity, Some(CLIENT_ID)).registry();
    let question = json!({ "client": "an-agent", "command": "mail.drafts.send", "args": { "draft_id": "draft-1", "body": "private words" } });

    // With no window looking, nobody can be asked.
    let unasked = registry
        .execute("app.confirmations.ask", question.clone())
        .await
        .unwrap();
    assert_eq!(unasked["allowed"], false);

    for allow in [true, false] {
        // The window looks, and so is known to be there.
        registry
            .execute("app.confirmations.list", json!({}))
            .await
            .unwrap();
        let asking = {
            let registry = registry.clone();
            let question = question.clone();
            tokio::spawn(async move {
                registry
                    .execute("app.confirmations.ask", question)
                    .await
                    .unwrap()
            })
        };
        let waiting = loop {
            let listed = registry
                .execute("app.confirmations.list", json!({}))
                .await
                .unwrap();
            if let Some(first) = listed["questions"].get(0) {
                break first.clone();
            }
            tokio::task::yield_now().await;
        };
        assert_eq!(waiting["client"], "an-agent");
        // What the person is shown has nothing private in it.
        assert_eq!(waiting["args"]["body"], "[13 characters]");

        let answered = registry
            .execute(
                "app.confirmations.answer",
                json!({ "id": waiting["id"], "allow": allow }),
            )
            .await
            .unwrap();
        assert_eq!(answered["answered"], true);
        assert_eq!(asking.await.unwrap()["allowed"], allow);
        // Answered, it is no longer waiting, and cannot be answered again.
        let after = registry
            .execute("app.confirmations.list", json!({}))
            .await
            .unwrap();
        assert!(after["questions"].as_array().unwrap().is_empty());
        let again = registry
            .execute(
                "app.confirmations.answer",
                json!({ "id": waiting["id"], "allow": true }),
            )
            .await
            .unwrap();
        assert_eq!(again["answered"], false);
    }
}
