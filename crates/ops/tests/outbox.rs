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
    client: Arc<GraphClient>,
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
        outbox: Outbox::new(client.clone(), db.clone()),
        client,
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
async fn a_send_that_may_have_been_carried_out_is_watched_and_then_handed_back_not_made_again() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    // The draft is made and filled in, and then the server falls over as
    // it is asked to send: an answer that does not say how far it got.
    f.graph.fail_after(2, [Fault::Unavailable]);

    let first = f.outbox.drain().await.unwrap();
    assert_eq!((first.sent, first.waiting, first.refused), (0, 1, 0));
    f.graph.clear_faults();

    // Still a draft a moment later. It may be about to go, so it is not
    // sent again; it is watched.
    let watching = f.outbox.drain().await.unwrap();
    assert_eq!(
        (watching.sent, watching.waiting, watching.refused),
        (0, 1, 0)
    );
    let held = f.db.draft(&draft.id).unwrap().unwrap();
    assert_eq!(held.state, "queued");
    assert!(held.error.unwrap().contains("checking"));
    assert!(f.graph.sent().is_empty());

    // Still a draft when the watch is over. Its writer is given it back.
    let done_watching =
        Outbox::new(f.client.clone(), f.db.clone()).confirming_within(std::time::Duration::ZERO);
    let handed_back = done_watching.drain().await.unwrap();
    assert_eq!(
        (handed_back.sent, handed_back.waiting, handed_back.refused),
        (0, 0, 1)
    );
    let back = f.db.draft(&draft.id).unwrap().unwrap();
    assert_eq!(back.state, "failed");
    assert!(back.error.unwrap().contains("Check Sent Items"));
    assert!(f.graph.sent().is_empty(), "it was sent unasked");

    // Sent again by its writer, it goes, once, as the draft it was.
    f.db.queue_draft(&draft.id).unwrap();
    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);
    assert_eq!(f.graph.sent().len(), 1);
    assert_eq!(f.graph.drafts_created(), 1);
}

#[tokio::test]
async fn a_send_that_was_accepted_late_is_known_for_sent_while_it_is_watched() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    f.graph.lose_next_send_answer();

    assert_eq!(f.outbox.drain().await.unwrap().waiting, 1);
    // The watch being over makes no difference to one that has gone.
    let later =
        Outbox::new(f.client.clone(), f.db.clone()).confirming_within(std::time::Duration::ZERO);
    assert_eq!(later.drain().await.unwrap().sent, 1);
    assert_eq!(f.graph.sent().len(), 1);
    assert!(f.db.drafts().unwrap().is_empty());
}

#[tokio::test]
async fn a_send_turned_away_as_too_busy_is_made_again_unasked() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    // Throttled for as long as the request is tried: turned away before
    // anything was done, which is certain.
    f.graph.fail_after(
        2,
        std::iter::repeat_n(
            Fault::Throttled {
                retry_after_secs: 0,
            },
            6,
        ),
    );

    let first = f.outbox.drain().await.unwrap();
    assert_eq!((first.sent, first.waiting), (0, 1));
    f.graph.clear_faults();

    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);
    assert_eq!(f.graph.sent().len(), 1);
}

#[tokio::test]
async fn a_new_draft_whose_making_was_never_heard_of_is_found_and_not_made_twice() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    f.graph.lose_next_draft_answer();

    let first = f.outbox.drain().await.unwrap();
    assert_eq!((first.sent, first.waiting), (0, 1));
    assert_eq!(f.graph.drafts_created(), 1);

    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);
    assert_eq!(f.graph.drafts_created(), 1, "a second draft was made");
    assert_eq!(f.graph.sent().len(), 1);
    // Nothing is left behind in the mailbox's drafts.
    let drafts = f.graph.folder_id("drafts");
    assert!(
        !f.graph
            .messages()
            .values()
            .any(|(folder, _, _)| *folder == drafts)
    );
}

#[tokio::test]
async fn sending_carries_on_from_wherever_it_was_stopped() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();

    // The server falls over at each step before the sending: creating the
    // draft, filling it in. Each time the outbox is run again.
    for outage in [0, 1] {
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

/// A draft with a subject and words and nobody to send it to, so that the
/// server takes its files and then will not send it.
fn to_nobody() -> DraftEdit {
    DraftEdit {
        subject: Some("Lunch".to_owned()),
        body: Some("Friday?".to_owned()),
        ..DraftEdit::default()
    }
}

#[tokio::test]
async fn a_send_taken_back_while_it_is_on_its_way_does_not_go() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    // The outbox has the draft in hand and is filling in the server's copy
    // when the send is cancelled and the words are changed.
    let held = f.graph.hold("PATCH /v1.0/me/messages/");

    let (drained, ()) = tokio::join!(f.outbox.drain(), async {
        held.reached().await;
        assert!(f.db.unqueue_draft(&draft.id).unwrap());
        let change = DraftEdit {
            body: Some("Not Friday after all.".to_owned()),
            ..DraftEdit::default()
        };
        assert!(f.db.edit_draft(&draft.id, &change).unwrap());
        held.release();
    });

    assert_eq!(drained.unwrap().sent, 0);
    assert!(f.graph.sent().is_empty(), "a cancelled message was sent");
    let kept = f.db.draft(&draft.id).unwrap().unwrap();
    assert_eq!(
        (kept.state.as_str(), kept.body.as_str()),
        ("editing", "Not Friday after all.")
    );

    // Sent again, it goes once, as it now reads.
    f.db.queue_draft(&draft.id).unwrap();
    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);
    assert_eq!(f.graph.sent().len(), 1);
    assert!(
        f.graph.sent()[0]
            .body_html
            .contains("Not Friday after all.")
    );
}

#[tokio::test]
async fn a_send_taken_back_and_queued_again_goes_as_it_was_last_written() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    let held = f.graph.hold("PATCH /v1.0/me/messages/");

    let (drained, ()) = tokio::join!(f.outbox.drain(), async {
        held.reached().await;
        assert!(f.db.unqueue_draft(&draft.id).unwrap());
        let change = DraftEdit {
            body: Some("Saturday?".to_owned()),
            ..DraftEdit::default()
        };
        assert!(f.db.edit_draft(&draft.id, &change).unwrap());
        assert!(f.db.queue_draft(&draft.id).unwrap());
        held.release();
    });

    // The pass that had the older words in hand does not send them.
    assert_eq!(drained.unwrap().sent, 0);
    assert!(f.graph.sent().is_empty(), "the older words were sent");

    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);
    assert_eq!(f.graph.sent().len(), 1);
    assert!(f.graph.sent()[0].body_html.contains("Saturday?"));
}

#[tokio::test]
async fn a_message_whose_sending_cannot_be_confirmed_is_kept() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    f.graph.lose_next_send_answer();
    assert_eq!(f.outbox.drain().await.unwrap().waiting, 1);
    // Whatever became of it, the message is no longer on the server to be
    // asked about.
    let server_id = f.db.draft(&draft.id).unwrap().unwrap().server_id.unwrap();
    f.graph.delete_message(&server_id);

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!((drained.sent, drained.refused), (0, 1));
    let kept = f.db.draft(&draft.id).unwrap().unwrap();
    assert_eq!(kept.state, "failed");
    assert_eq!(kept.body, "Friday?");
    assert!(
        kept.error.as_deref().unwrap().contains("Check Sent Items"),
        "{:?}",
        kept.error
    );
}

#[tokio::test]
async fn a_file_taken_off_a_draft_is_taken_off_the_servers_copy_too() {
    let f = fixture().await;
    let draft = f.db.create_draft("new", None, &to_nobody()).unwrap();
    let wrong =
        f.db.attach_to_draft(&draft.id, "wrong.txt", "text/plain", b"Not this one.")
            .unwrap()
            .unwrap();
    f.db.attach_to_draft(&draft.id, "right.txt", "text/plain", b"This one.")
        .unwrap();
    f.db.attach_inline_to_draft(&draft.id, "logo.png", "image/png", b"picture")
        .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    // Both files reach the server's copy before it refuses to send.
    assert_eq!(f.outbox.drain().await.unwrap().refused, 1);
    let server_id = f.db.draft(&draft.id).unwrap().unwrap().server_id.unwrap();
    assert_eq!(f.graph.attached(&server_id).len(), 3);

    let picture = f.db.draft_files(&draft.id).unwrap()[2].id;
    assert!(f.db.detach_from_draft(&draft.id, wrong.id).unwrap());
    assert!(f.db.detach_from_draft(&draft.id, picture).unwrap());
    f.db.edit_draft(&draft.id, &to("jo@example.test")).unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);

    let sent = &f.graph.sent()[0];
    let names: Vec<&str> = sent.attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["right.txt"], "a file taken off was sent");
}

#[tokio::test]
async fn a_file_taken_off_after_a_lost_answer_is_not_sent() {
    let f = fixture().await;
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    let wrong =
        f.db.attach_to_draft(&draft.id, "notes.txt", "text/plain", b"Not this one.")
            .unwrap()
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    // The file is attached, and the answer saying so is lost.
    f.graph.lose_next_attachment_answer();
    assert_eq!(f.outbox.drain().await.unwrap().waiting, 1);

    // The send is taken back, and a file of the same name put in its place.
    assert!(f.db.unqueue_draft(&draft.id).unwrap());
    assert!(f.db.detach_from_draft(&draft.id, wrong.id).unwrap());
    f.db.attach_to_draft(&draft.id, "notes.txt", "text/plain", b"This one, yes.")
        .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);

    let sent = &f.graph.sent()[0];
    assert_eq!(sent.attachments.len(), 1, "{:?}", sent.attachments);
    assert_eq!(sent.attachments[0].content, b"This one, yes.");
}

#[tokio::test]
async fn files_go_again_when_the_servers_copy_has_to_be_made_again() {
    let f = fixture().await;
    let draft = f.db.create_draft("new", None, &to_nobody()).unwrap();
    f.db.attach_to_draft(&draft.id, "notes.txt", "text/plain", b"Some notes.")
        .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    assert_eq!(f.outbox.drain().await.unwrap().refused, 1);
    let server_id = f.db.draft(&draft.id).unwrap().unwrap().server_id.unwrap();
    assert_eq!(f.graph.attached(&server_id).len(), 1);

    // The server's copy, file and all, is deleted in another mail program.
    f.graph.delete_message(&server_id);
    f.db.edit_draft(&draft.id, &to("jo@example.test")).unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    let mut passes = 0;
    while f.graph.sent().is_empty() {
        f.outbox.drain().await.unwrap();
        passes += 1;
        assert!(passes < 5, "never sent");
    }

    let sent = &f.graph.sent()[0];
    assert_eq!(sent.attachments.len(), 1, "the file was left behind");
    assert_eq!(sent.attachments[0].content, b"Some notes.");
}

#[tokio::test]
async fn a_file_already_on_the_draft_is_not_taken_for_one_attached_here() {
    let f = fixture().await;
    // A draft written in another mail program, with a file on it.
    let mut elsewhere = FakeMessage::new("Report", "Robin", "robin@example.test", "").attach(
        "report.pdf",
        "application/pdf",
        b"The first report",
    );
    elsewhere.is_draft = true;
    elsewhere.to = vec![("Jo".to_owned(), "jo@example.test".to_owned())];
    let server_id = f.graph.add_message(&f.graph.folder_id("drafts"), elsewhere);
    let draft =
        f.db.adopt_draft(&server_id, &to("jo@example.test"))
            .unwrap();
    // Carried on with here, and given a second file of the same name and size.
    f.db.attach_to_draft(
        &draft.id,
        "report.pdf",
        "application/pdf",
        b"Another report..",
    )
    .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    // The first try at attaching it does not reach the server.
    f.graph.drop_next_attachment();
    assert_eq!(f.outbox.drain().await.unwrap().waiting, 1);

    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);

    let sent = &f.graph.sent()[0];
    let contents: Vec<&[u8]> = sent
        .attachments
        .iter()
        .map(|a| a.content.as_slice())
        .collect();
    assert_eq!(
        contents,
        [&b"The first report"[..], &b"Another report.."[..]],
        "the file attached here was left behind"
    );
}

#[tokio::test]
async fn a_message_moved_here_and_deleted_for_good_elsewhere_is_forgotten() {
    let f = fixture().await;
    let archive = f.graph.folder_id("archive");
    f.db.queue_edit("a", &f.ids[..1], &MessageEdit::Move(archive))
        .unwrap();
    f.db.queue_edit("b", &f.ids[..1], &MessageEdit::Read(true))
        .unwrap();
    f.graph.delete_message(&f.ids[0]);

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!((drained.waiting, drained.refused), (0, 0));
    assert!(
        f.db.message(&f.ids[0]).unwrap().is_none(),
        "it is still shown"
    );
    let (queued, failed) = f.db.outbox().unwrap();
    assert_eq!((queued, failed.len()), (0, 0));
}

#[tokio::test]
async fn a_move_to_a_folder_deleted_elsewhere_is_taken_back_and_reported() {
    let f = fixture().await;
    let inbox = f.graph.folder_id("inbox");
    let gone = f.graph.add_folder("Projects", None);
    f.db.queue_edit("a", &f.ids[..1], &MessageEdit::Move(gone.clone()))
        .unwrap();
    f.graph.remove_folder(&gone);

    let drained = f.outbox.drain().await.unwrap();

    assert_eq!((drained.sent, drained.refused), (0, 1));
    assert_eq!(f.db.message(&f.ids[0]).unwrap().unwrap().folder_id, inbox);
    let (_, failed) = f.db.outbox().unwrap();
    assert_eq!(failed[0].message_id, f.ids[0]);
}

#[tokio::test]
async fn taking_the_day_off_a_flag_reaches_the_server() {
    let f = fixture().await;
    let one = &f.ids[..1];
    f.db.queue_edit("a", one, &MessageEdit::Flag("flagged".to_owned()))
        .unwrap();
    f.db.queue_edit("a", one, &MessageEdit::FlagDue("2026-10-20".to_owned()))
        .unwrap();
    f.outbox.drain().await.unwrap();
    assert_eq!(f.graph.flag_due(&f.ids[0]).as_deref(), Some("2026-10-20"));

    f.db.queue_edit("b", one, &MessageEdit::FlagDue(String::new()))
        .unwrap();
    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);

    assert_eq!(f.graph.flag_due(&f.ids[0]), None);
    assert_eq!(f.graph.flag(&f.ids[0]).as_deref(), Some("flagged"));
}

#[tokio::test]
async fn a_flags_day_is_sent_as_that_day_where_its_writer_is() {
    let f = fixture().await;
    let one = &f.ids[..1];
    let client = || {
        Arc::new(
            GraphClient::new(
                f.graph.base_url(),
                Arc::new(Token("token.Mail.ReadWrite")),
                RetryPolicy::impatient(),
            )
            .unwrap(),
        )
    };
    f.db.queue_edit("a", one, &MessageEdit::Flag("flagged".to_owned()))
        .unwrap();
    f.db.queue_edit("a", one, &MessageEdit::FlagDue("2026-10-20".to_owned()))
        .unwrap();

    // Ten hours ahead of UTC all year: the day begins the evening before.
    let brisbane = Outbox::new(client(), f.db.clone()).in_zone("Australia/Brisbane");
    brisbane.drain().await.unwrap();
    assert_eq!(
        f.graph.flag_due_at(&f.ids[0]).as_deref(),
        Some("2026-10-19T14:00:00Z")
    );

    // Four hours behind in October: it begins in the small hours.
    f.db.queue_edit("b", one, &MessageEdit::FlagDue("2026-10-21".to_owned()))
        .unwrap();
    let new_york = Outbox::new(client(), f.db.clone()).in_zone("America/New_York");
    new_york.drain().await.unwrap();
    assert_eq!(
        f.graph.flag_due_at(&f.ids[0]).as_deref(),
        Some("2026-10-21T04:00:00Z")
    );
}

#[tokio::test]
async fn a_draft_thrown_away_while_its_copy_is_being_made_leaves_no_copy_behind() {
    let f = fixture().await;
    let drafts = f.graph.folder_id("drafts");
    let in_drafts = || {
        f.graph
            .messages()
            .values()
            .filter(|(folder, _, _)| *folder == drafts)
            .count()
    };
    let draft =
        f.db.create_draft("new", None, &to("jo@example.test"))
            .unwrap();
    // The request that makes the server's copy is on its way when the
    // draft is thrown away here.
    let held = f.graph.hold("POST /v1.0/me/messages");

    let (drained, ()) = tokio::join!(f.outbox.drain(), async {
        held.reached().await;
        assert!(f.db.discard_draft(&draft.id).unwrap());
        held.release();
    });
    assert_eq!(drained.unwrap().saved, 0);
    f.outbox.drain().await.unwrap();

    assert_eq!(in_drafts(), 0, "a copy nobody has was left in Drafts");
    let deleted = f.graph.folder_id("deleteditems");
    assert_eq!(
        f.graph
            .messages()
            .values()
            .filter(|(folder, _, _)| *folder == deleted)
            .count(),
        1
    );
    assert!(f.db.draft_discards().unwrap().is_empty());
}

#[tokio::test]
async fn a_large_file_taken_off_a_draft_is_taken_off_the_servers_copy_by_its_id() {
    let f = fixture().await;
    let draft = f.db.create_draft("new", None, &to_nobody()).unwrap();
    let big: Vec<u8> = (0..3 * 1024 * 1024 + 5).map(|n| (n % 251) as u8).collect();
    let file =
        f.db.attach_to_draft(&draft.id, "survey.zip", "application/zip", &big)
            .unwrap()
            .unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    assert_eq!(f.outbox.drain().await.unwrap().refused, 1);
    // The answer to the last piece said which attachment it had become.
    let server_id = f.db.draft(&draft.id).unwrap().unwrap().server_id.unwrap();
    assert_eq!(
        f.db.draft_files(&draft.id).unwrap()[0].attachment_id,
        Some(f.graph.attached(&server_id)[0].id.clone())
    );

    assert!(f.db.detach_from_draft(&draft.id, file.id).unwrap());
    f.db.edit_draft(&draft.id, &to("jo@example.test")).unwrap();
    f.db.queue_draft(&draft.id).unwrap();
    assert_eq!(f.outbox.drain().await.unwrap().sent, 1);

    assert!(f.graph.sent()[0].attachments.is_empty());
}
