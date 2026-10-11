#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The outbox against the fake Graph server.

use std::sync::Arc;

use brolga_fake_graph::{FakeGraph, FakeMessage, Fault};
use brolga_graph::{GraphClient, GraphError, RetryPolicy, TokenSource};
use brolga_ops::{Drained, Outbox};
use brolga_store::{AccountDb, MessageChange, MessageEdit, Round};
use futures::future::BoxFuture;

struct Token(&'static str);

impl TokenSource for Token {
    fn token(&self) -> BoxFuture<'_, Result<String, GraphError>> {
        Box::pin(async { Ok(self.0.to_owned()) })
    }
}

struct Fixture {
    graph: FakeGraph,
    db: Arc<AccountDb>,
    outbox: Outbox,
    /// Two messages, known to the server and to this computer, both unread.
    ids: Vec<String>,
}

async fn fixture_with(token: &'static str) -> Fixture {
    let graph = FakeGraph::start().await.unwrap();
    let inbox = graph.folder_id("inbox");
    let ids: Vec<String> = ["One", "Two"]
        .iter()
        .map(|subject| {
            graph.add_message(
                &inbox,
                FakeMessage::new(subject, "Sam", "sam@example.test", "2026-10-01T09:00:00Z"),
            )
        })
        .collect();

    let db = Arc::new(AccountDb::in_memory().unwrap());
    let known: Vec<MessageChange> = ids
        .iter()
        .map(|id| MessageChange {
            id: id.clone(),
            is_read: Some(false),
            flag_status: Some("notFlagged".to_owned()),
            ..MessageChange::default()
        })
        .collect();
    db.apply_messages(&inbox, &known, &[], "next", Round::Finished)
        .unwrap();

    let client = Arc::new(
        GraphClient::new(
            graph.base_url(),
            Arc::new(Token(token)),
            RetryPolicy::impatient(),
        )
        .unwrap(),
    );

    Fixture {
        outbox: Outbox::new(client, db.clone()),
        graph,
        db,
        ids,
    }
}

async fn fixture() -> Fixture {
    fixture_with("token.Mail.ReadWrite").await
}

impl Fixture {
    fn read_on_server(&self, id: &str) -> bool {
        self.graph.messages()[id].2
    }

    fn read_here(&self, id: &str) -> bool {
        self.db.message(id).unwrap().unwrap().is_read
    }
}

#[tokio::test]
async fn queued_changes_reach_the_server() {
    let f = fixture().await;
    f.db.queue_edit("a", &f.ids, &MessageEdit::Read(true))
        .unwrap();
    f.db.queue_edit("b", &f.ids[..1], &MessageEdit::Flag("flagged".to_owned()))
        .unwrap();

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!(
        drained,
        Drained {
            sent: 3,
            waiting: 0,
            refused: 0
        }
    );
    assert!(f.read_on_server(&f.ids[0]) && f.read_on_server(&f.ids[1]));
    assert_eq!(f.graph.flag(&f.ids[0]).as_deref(), Some("flagged"));
    assert_eq!(f.db.outbox().unwrap().0, 0);
    // Nothing is left to send a second time.
    assert_eq!(f.outbox.drain().await.unwrap(), Drained::default());
}

#[tokio::test]
async fn a_change_that_cannot_be_sent_now_waits_and_goes_later() {
    let f = fixture().await;
    f.db.queue_edit("a", &f.ids[..1], &MessageEdit::Read(true))
        .unwrap();
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 20));

    let while_down = f.outbox.drain().await.unwrap();

    assert_eq!(
        while_down,
        Drained {
            sent: 0,
            waiting: 1,
            refused: 0
        }
    );
    // It still shows as read here, and is still owed to the server.
    assert!(f.read_here(&f.ids[0]));
    assert!(!f.read_on_server(&f.ids[0]));

    f.graph.fail_next([]);
    while f.outbox.drain().await.unwrap().sent == 0 {}
    assert!(f.read_on_server(&f.ids[0]));
}

#[tokio::test]
async fn changes_to_one_message_arrive_in_the_order_they_were_made() {
    let f = fixture().await;
    let one = &f.ids[..1];
    f.db.queue_edit("a", one, &MessageEdit::Read(true)).unwrap();
    f.db.queue_edit("b", one, &MessageEdit::Read(false))
        .unwrap();
    f.db.queue_edit("c", one, &MessageEdit::Read(true)).unwrap();
    f.db.queue_edit("d", &f.ids[1..], &MessageEdit::Read(true))
        .unwrap();
    // The first change fails on every try it is given; the rest of that
    // message's changes must wait behind it, not overtake it.
    f.graph
        .fail_next(std::iter::repeat_n(Fault::Unavailable, 6));

    let first = f.outbox.drain().await.unwrap();
    assert_eq!(first.waiting, 3, "{first:?}");
    assert_eq!(first.sent, 1, "the other message need not wait");

    let rest = f.outbox.drain().await.unwrap();
    assert_eq!(rest.sent, 3);
    assert!(f.read_on_server(&f.ids[0]));
}

#[tokio::test]
async fn a_change_the_server_refuses_is_taken_back_and_reported() {
    let f = fixture().await;
    f.graph.refuse_updates_to(&f.ids[0]);
    f.db.queue_edit("a", &f.ids, &MessageEdit::Read(true))
        .unwrap();

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!(
        drained,
        Drained {
            sent: 1,
            waiting: 0,
            refused: 1
        }
    );
    assert!(!f.read_here(&f.ids[0]));
    assert!(f.read_here(&f.ids[1]));
    let (queued, failed) = f.db.outbox().unwrap();
    assert_eq!(queued, 0);
    assert_eq!(failed[0].message_id, f.ids[0]);
    assert!(
        failed[0].error.contains("cannot be changed"),
        "{}",
        failed[0].error
    );
}

#[tokio::test]
async fn a_sign_in_that_may_only_read_cannot_change_mail() {
    let f = fixture_with("token.Mail.Read").await;
    f.db.queue_edit("a", &f.ids[..1], &MessageEdit::Read(true))
        .unwrap();

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!(drained.refused, 1);
    assert!(!f.read_on_server(&f.ids[0]));
    assert!(!f.read_here(&f.ids[0]));
}

#[tokio::test]
async fn a_change_to_a_message_since_deleted_is_let_go() {
    let f = fixture().await;
    f.db.queue_edit("a", &f.ids[..1], &MessageEdit::Read(true))
        .unwrap();
    f.graph.delete_message(&f.ids[0]);

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!(
        drained,
        Drained {
            sent: 1,
            waiting: 0,
            refused: 0
        }
    );
    assert!(f.db.outbox().unwrap().1.is_empty());
}

#[tokio::test]
async fn a_move_reaches_the_server() {
    let f = fixture().await;
    let archive = f.graph.folder_id("archive");
    f.db.queue_edit("a", &f.ids[..1], &MessageEdit::Move(archive.clone()))
        .unwrap();

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!(drained.sent, 1);
    assert_eq!(f.graph.messages()[&f.ids[0]].0, archive);
}

#[tokio::test]
async fn a_move_whose_answer_was_lost_is_not_made_twice() {
    let f = fixture().await;
    let archive = f.graph.folder_id("archive");
    f.db.queue_edit("a", &f.ids[..1], &MessageEdit::Move(archive.clone()))
        .unwrap();
    f.graph.lose_next_move_answer();

    let first = f.outbox.drain().await.unwrap();
    assert_eq!(
        first,
        Drained {
            sent: 0,
            waiting: 1,
            refused: 0
        }
    );

    let second = f.outbox.drain().await.unwrap();
    assert_eq!(second.sent, 1);
    assert_eq!(f.graph.moves(), 1, "the move was sent again");
    assert_eq!(f.graph.messages()[&f.ids[0]].0, archive);
}
