#![allow(clippy::unwrap_used, clippy::expect_used)]

//! What the client will and will not ask a server, whatever it is handed.

use std::sync::Arc;

use brolga_fake_graph::{FakeGraph, FakeMessage};
use brolga_graph::{GraphClient, GraphError, Next, RetryPolicy, TokenSource};
use futures::future::BoxFuture;

struct Token;

impl TokenSource for Token {
    fn token(&self) -> BoxFuture<'_, Result<String, GraphError>> {
        Box::pin(async { Ok("token.Mail.ReadWrite".to_owned()) })
    }
}

fn client(graph: &FakeGraph) -> GraphClient {
    GraphClient::new(graph.base_url(), Arc::new(Token), RetryPolicy::impatient()).unwrap()
}

#[tokio::test]
async fn an_id_cannot_turn_a_request_into_one_for_something_else() {
    let graph = FakeGraph::start().await.unwrap();
    let client = client(&graph);
    let draft = graph.add_message(
        &graph.folder_id("drafts"),
        FakeMessage::new("Draft", "Me", "me@example.test", "2026-10-01T09:00:00Z"),
    );

    for hostile in [
        format!("../messages/{draft}/send#"),
        format!("../messages/{draft}/send?"),
        format!("x/../../messages/{draft}/send#"),
        format!("..\\messages\\{draft}\\send#"),
        format!("%2e%2e/messages/{draft}/send#"),
        "..".to_owned(),
        ".".to_owned(),
        String::new(),
    ] {
        let _ = client.dismiss_reminder(&hostile).await;
        let _ = client
            .snooze_reminder(&hostile, "2026-10-01T09:00:00Z")
            .await;
        let _ = client.attachments(&hostile).await;
        let _ = client.event(&hostile).await;
        let _ = client.delete_event(&hostile).await;
    }

    assert!(graph.sent().is_empty());
    for request in graph.requests() {
        let (method, path) = request.split_once(' ').unwrap();
        let rest = path.strip_prefix("/v1.0/me/").unwrap();
        let parts: Vec<&str> = rest.split('/').collect();
        let expected = matches!(
            (method, parts.as_slice()),
            ("POST", ["events", _, "dismissReminder" | "snoozeReminder"])
                | ("GET", ["messages", _, "attachments"])
                | ("GET" | "DELETE", ["events", _])
        );
        assert!(expected, "{request}");
    }
}

#[tokio::test]
async fn a_link_to_another_server_is_not_followed_and_is_given_no_token() {
    let graph = FakeGraph::start().await.unwrap();
    let elsewhere = FakeGraph::start().await.unwrap();
    let client = client(&graph);
    let inbox = graph.folder_id("inbox");
    graph.add_messages(&inbox, 3);
    graph.write_links_to(&elsewhere.base_url());

    let first = client.messages_delta(&inbox, None, 1).await.unwrap();
    let Next::Page(link) = first.next else {
        panic!("one page was all of it");
    };
    assert!(link.starts_with(&elsewhere.base_url()), "{link}");
    let second = client.messages_delta(&inbox, Some(&link), 1).await;

    assert!(matches!(second, Err(GraphError::Decode(_))), "{second:?}");
    assert!(
        elsewhere.requests().is_empty(),
        "{:?}",
        elsewhere.requests()
    );
}

#[tokio::test]
async fn a_probe_stays_under_the_mailbox_it_is_for() {
    let graph = FakeGraph::start().await.unwrap();
    let client = client(&graph);

    for path in [
        "/../users/someone@example.test/messages",
        "/messages/../../users",
        "/..",
        "/%2e%2e/users",
        "/.\t./users",
        "/..\\users",
        "messages",
    ] {
        let answer = client.probe(path).await;
        assert!(matches!(answer, Err(GraphError::Decode(_))), "{path}");
    }
    assert!(graph.requests().is_empty(), "{:?}", graph.requests());

    let (status, _) = client.probe("/mailFolders/inbox?$select=id").await.unwrap();
    assert_eq!(status, 200);
    assert_eq!(graph.requests(), ["GET /v1.0/me/mailFolders/inbox"]);
}
