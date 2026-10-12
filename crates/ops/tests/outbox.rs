#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The outbox against the fake Graph server.

use std::sync::Arc;

use brolga_fake_graph::{FakeGraph, FakeMessage, Fault};
use brolga_graph::{GraphClient, GraphError, RetryPolicy, TokenSource};
use brolga_ops::{Drained, Outbox};
use brolga_store::{AccountDb, DraftEdit, MessageChange, MessageEdit, Round};
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
    fixture_with("token.Mail.ReadWrite,Mail.Send").await
}

fn to(address: &str) -> DraftEdit {
    DraftEdit {
        to_recipients: Some(format!(r#"[{{"name":"Jo","address":"{address}"}}]"#)),
        subject: Some("Lunch".to_owned()),
        body: Some("Friday?".to_owned()),
        ..DraftEdit::default()
    }
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
            refused: 0,
            saved: 0
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
            refused: 0,
            saved: 0
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
            refused: 1,
            saved: 0
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
            refused: 0,
            saved: 0
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
            refused: 0,
            saved: 0
        }
    );

    let second = f.outbox.drain().await.unwrap();
    assert_eq!(second.sent, 1);
    assert_eq!(f.graph.moves(), 1, "the move was sent again");
    assert_eq!(f.graph.messages()[&f.ids[0]].0, archive);
}

#[tokio::test]
async fn a_queued_message_is_sent_with_what_was_written() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!(drained.sent, 1);
    let sent = f.graph.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].subject, "Lunch");
    assert_eq!(
        sent[0].to,
        [("Jo".to_owned(), "jo@example.test".to_owned())]
    );
    assert_eq!(
        sent[0].body_html,
        "<html><body><p>Friday?</p></body></html>"
    );
    assert!(f.db.drafts().unwrap().is_empty());
}

#[tokio::test]
async fn a_reply_goes_out_above_the_original_and_in_its_conversation() {
    let f = fixture().await;
    let start = DraftEdit {
        to_recipients: Some(r#"[{"name":"Sam","address":"sam@example.test"}]"#.to_owned()),
        subject: Some("RE: One".to_owned()),
        body: Some("Yes.".to_owned()),
        ..DraftEdit::default()
    };
    let draft = f.db.create_draft("reply", Some(&f.ids[0]), &start).unwrap();
    f.db.queue_draft(&draft.id).unwrap();

    f.outbox.drain().await.unwrap();

    let sent = &f.graph.sent()[0];
    assert!(
        sent.body_html.starts_with("<html><body><p>Yes.</p><hr>"),
        "{}",
        sent.body_html
    );
    assert!(sent.body_html.contains("One"));
    assert_eq!(
        sent.conversation.as_deref(),
        Some(format!("conversation-{}", f.ids[0]).as_str())
    );
    // The message answered is marked as answered, and the other is not.
    let did = |id: &str| f.db.message(id).unwrap().unwrap().last_action;
    assert_eq!(
        (did(&f.ids[0]), did(&f.ids[1])),
        ("replied".to_owned(), String::new())
    );

    let on =
        f.db.create_draft("forward", Some(&f.ids[1]), &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&on.id).unwrap();
    f.outbox.drain().await.unwrap();
    assert_eq!(did(&f.ids[1]), "forwarded");
}

#[tokio::test]
async fn a_message_is_not_sent_twice_when_the_answer_to_sending_is_lost() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    f.graph.lose_next_send_answer();

    let first = f.outbox.drain().await.unwrap();
    // It may or may not have gone, so it is still owed and still kept.
    assert_eq!(
        first,
        Drained {
            sent: 0,
            waiting: 1,
            refused: 0,
            saved: 0
        }
    );
    assert_eq!(f.db.queued_drafts().unwrap().len(), 1);

    let second = f.outbox.drain().await.unwrap();
    assert_eq!(second.sent, 1);
    assert_eq!(f.graph.sent().len(), 1, "the message was sent twice");
    assert!(f.db.drafts().unwrap().is_empty());
}

#[tokio::test]
async fn sending_carries_on_from_wherever_it_was_stopped() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();

    // The server falls over at each step in turn: creating the draft,
    // filling it in, sending it. Each time the outbox is run again.
    for outage in [0, 1, 2] {
        f.graph
            .fail_after(outage, std::iter::repeat_n(Fault::Unavailable, 12));
        let mut passes = 0;
        while !f.db.queued_drafts().unwrap().is_empty() {
            f.outbox.drain().await.unwrap();
            passes += 1;
            assert!(passes < 20, "never sent");
        }
        assert_eq!(f.graph.sent().len(), outage + 1);
        assert_eq!(
            f.graph.drafts_created() as usize,
            outage + 1,
            "a draft was made twice"
        );

        let again =
            f.db.create_draft("new", None, &to("jo@example.test"))
                .unwrap();
        f.db.queue_draft(&again.id).unwrap();
        f.graph.clear_faults();
    }
}

#[tokio::test]
async fn a_message_the_server_will_not_send_comes_back_to_be_put_right() {
    let f = fixture().await;
    let nobody = DraftEdit {
        subject: Some("To nobody".to_owned()),
        body: Some("Hello?".to_owned()),
        ..DraftEdit::default()
    };
    let draft = f.db.create_draft("new", None, &nobody).unwrap();
    f.db.queue_draft(&draft.id).unwrap();

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!(drained.refused, 1);
    assert!(f.graph.sent().is_empty());
    let back = f.db.draft(&draft.id).unwrap().unwrap();
    assert_eq!(back.state, "failed");
    assert_eq!(back.body, "Hello?");
    assert!(back.error.unwrap().contains("recipient"));

    // Put right, it goes, as the same draft on the server and not a second.
    f.db.edit_draft(&draft.id, &to("jo@example.test")).unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);
    assert_eq!(f.graph.drafts_created(), 1);
    assert_eq!(f.graph.sent()[0].subject, "Lunch");
}

#[tokio::test]
async fn a_sign_in_that_may_not_send_does_not() {
    let f = fixture_with("token.Mail.ReadWrite").await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!(drained.refused, 1);
    assert!(f.graph.sent().is_empty());
    assert_eq!(f.db.draft(&draft.id).unwrap().unwrap().state, "failed");
}

#[tokio::test]
async fn files_go_with_the_message_each_once() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.attach_to_draft(&draft.id, "notes.txt", "text/plain", b"Some notes.")
        .unwrap();
    f.db.attach_to_draft(&draft.id, "more.txt", "text/plain", b"More.")
        .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    // The first file is attached, and the answer to that is lost.
    f.graph.lose_next_attachment_answer();

    let first = f.outbox.drain().await.unwrap();
    assert_eq!(first.waiting, 1);
    assert!(f.graph.sent().is_empty());

    let second = f.outbox.drain().await.unwrap();
    assert_eq!(second.sent, 1);
    let sent = &f.graph.sent()[0];
    let names: Vec<&str> = sent.attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        ["notes.txt", "more.txt"],
        "a file went twice or not at all"
    );
    assert_eq!(sent.attachments[0].content, b"Some notes.");
    assert_eq!(sent.attachments[0].content_type, "text/plain");
}

#[tokio::test]
async fn a_large_file_goes_up_in_pieces_and_arrives_whole() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    // Seven megabytes and a bit: three pieces, the last a short one.
    let big: Vec<u8> = (0..7 * 1024 * 1024 + 123)
        .map(|n| (n % 251) as u8)
        .collect();
    f.db.attach_to_draft(&draft.id, "survey.zip", "application/zip", &big)
        .unwrap();
    f.db.attach_to_draft(&draft.id, "note.txt", "text/plain", b"Small.")
        .unwrap();
    f.db.queue_draft(&draft.id).unwrap();

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!(
        drained.sent,
        1,
        "{:?}",
        f.db.draft(&draft.id).unwrap().map(|d| d.error)
    );
    let sent = &f.graph.sent()[0];
    assert_eq!(sent.attachments.len(), 2);
    assert_eq!(sent.attachments[0].name, "survey.zip");
    assert!(
        sent.attachments[0].content == big,
        "the file did not arrive as it left"
    );
    assert_eq!(sent.attachments[1].content, b"Small.");
}

#[tokio::test]
async fn a_draft_being_written_is_kept_on_the_server_and_brought_up_to_date() {
    let f = fixture().await;
    let drafts = f.graph.folder_id("drafts");
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();

    assert_eq!(f.outbox.drain().await.unwrap().saved, 1);
    let on_server = |subject: &str| {
        f.graph
            .messages()
            .values()
            .filter(|(folder, s, _)| *folder == drafts && s == subject)
            .count()
    };
    assert_eq!(on_server("Lunch"), 1);
    // Nothing has changed, so nothing is sent again.
    assert_eq!(f.outbox.drain().await.unwrap().saved, 0);

    let change = DraftEdit {
        subject: Some("Lunch on Friday".to_owned()),
        ..DraftEdit::default()
    };
    f.db.edit_draft(&draft.id, &change).unwrap();
    assert_eq!(f.outbox.drain().await.unwrap().saved, 1);
    assert_eq!((on_server("Lunch"), on_server("Lunch on Friday")), (0, 1));
    assert_eq!(f.graph.drafts_created(), 1, "a second copy was made");

    // Sent, it is the same message that goes, and nothing is left in Drafts.
    f.db.queue_draft(&draft.id).unwrap();
    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);
    assert_eq!(f.graph.sent()[0].subject, "Lunch on Friday");
    assert_eq!(on_server("Lunch on Friday"), 0);
    assert_eq!(f.graph.drafts_created(), 1);
}

#[tokio::test]
async fn a_draft_thrown_away_elsewhere_is_still_here_and_can_still_be_sent() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.outbox.drain().await.unwrap();
    let server_id = f.db.draft(&draft.id).unwrap().unwrap().server_id.unwrap();

    // Deleted in another mail program.
    f.graph.delete_message(&server_id);
    let change = DraftEdit {
        body: Some("Still wanted.".to_owned()),
        ..DraftEdit::default()
    };
    f.db.edit_draft(&draft.id, &change).unwrap();
    f.outbox.drain().await.unwrap();
    assert_eq!(
        f.db.draft(&draft.id).unwrap().unwrap().body,
        "Still wanted."
    );

    f.db.queue_draft(&draft.id).unwrap();
    let mut passes = 0;
    while f.graph.sent().is_empty() {
        f.outbox.drain().await.unwrap();
        passes += 1;
        assert!(passes < 5, "never sent");
    }
    assert!(f.graph.sent()[0].body_html.contains("Still wanted."));
    assert_eq!(f.graph.sent().len(), 1);
}
