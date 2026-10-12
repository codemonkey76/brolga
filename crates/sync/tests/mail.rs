#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Mail sync against the fake Graph server, including the failures a real
//! one only produces now and then. Every test ends the same way: what is in
//! the local database must be exactly what is on the server.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use brolga_fake_graph::{FakeGraph, FakeMessage, Fault};
use brolga_graph::{GraphClient, GraphError, RetryPolicy, TokenSource};
use brolga_store::{AccountDb, Order};
use brolga_sync::{MailSync, SyncError};
use futures::future::BoxFuture;

struct FixedToken;

impl TokenSource for FixedToken {
    fn token(&self) -> BoxFuture<'_, Result<String, GraphError>> {
        Box::pin(async { Ok("token.Mail.ReadWrite".to_owned()) })
    }
}

struct Fixture {
    graph: FakeGraph,
    db: Arc<AccountDb>,
    inbox: String,
}

impl Fixture {
    async fn new() -> Self {
        let graph = FakeGraph::start().await.unwrap();
        let inbox = graph.folder_id("inbox");

        Self {
            graph,
            db: Arc::new(AccountDb::in_memory().unwrap()),
            inbox,
        }
    }

    /// A sync engine as a freshly started Brolga would have it: the same
    /// database, and nothing carried over in memory.
    fn sync(&self, page_size: u32) -> MailSync {
        let client = GraphClient::new(
            self.graph.base_url(),
            Arc::new(FixedToken),
            RetryPolicy::impatient(),
        )
        .unwrap();

        MailSync::new(Arc::new(client), self.db.clone()).with_page_size(page_size)
    }

    /// Every message in the local database: id to (folder, subject, read).
    fn local(&self) -> BTreeMap<String, (String, String, bool)> {
        let mut all = BTreeMap::new();
        // Every folder the server has ever had, so that a message left
        // behind in a folder that has gone would still be found.
        for folder in self.db.folders().unwrap() {
            for m in self.db.messages(&folder.id, 0, u32::MAX).unwrap() {
                let was = all.insert(m.id.clone(), (m.folder_id, m.subject, m.is_read));
                assert!(was.is_none(), "{} is in the database twice", m.id);
            }
        }

        all
    }

    fn assert_in_step(&self) {
        let (local, server) = (self.local(), self.graph.messages());
        let missing: Vec<_> = server
            .keys()
            .filter(|id| !local.contains_key(*id))
            .collect();
        let extra: Vec<_> = local
            .keys()
            .filter(|id| !server.contains_key(*id))
            .collect();

        assert!(
            missing.is_empty() && extra.is_empty(),
            "lost {missing:?}, kept {extra:?}"
        );
        assert_eq!(local, server);

        let folders: BTreeMap<String, String> = self
            .db
            .folders()
            .unwrap()
            .into_iter()
            .map(|f| (f.id, f.name))
            .collect();
        let on_server: BTreeMap<String, String> = self
            .graph
            .folders()
            .into_iter()
            .map(|(id, (name, _))| (id, name))
            .collect();
        assert_eq!(folders, on_server);
    }
}

fn message(subject: &str, received: &str) -> FakeMessage {
    FakeMessage::new(subject, "Sam Taylor", "sam@example.test", received)
}

#[tokio::test]
async fn a_first_sync_brings_down_the_folders_and_every_message() {
    let f = Fixture::new().await;
    f.graph.add_messages(&f.inbox, 250);
    let projects = f.graph.add_folder("Projects", None);
    f.graph.add_messages(&projects, 30);

    let synced = f.sync(20).sync_all().await.unwrap();

    f.assert_in_step();
    assert_eq!(synced.changed, 280 + 8);
    let folders = f.db.folders().unwrap();
    assert_eq!(folders[0].well_known.as_deref(), Some("inbox"));
    assert_eq!((folders[0].total, folders[0].unread), (250, 250));
    assert_eq!(
        folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>()[..4],
        ["Inbox", "Drafts", "Sent Items", "Deleted Items"]
    );
}

#[tokio::test]
async fn the_newest_messages_arrive_first() {
    let f = Fixture::new().await;
    f.graph
        .add_message(&f.inbox, message("Old", "2026-01-01T09:00:00Z"));
    f.graph
        .add_message(&f.inbox, message("Newest", "2026-10-01T09:00:00Z"));
    f.graph
        .add_message(&f.inbox, message("Middle", "2026-06-01T09:00:00Z"));
    let sync = f.sync(1);
    sync.sync_folders().await.unwrap();

    // One page of one message, then stopped: as when the app is closed early.
    let first_page = GraphClient::new(
        f.graph.base_url(),
        Arc::new(FixedToken),
        RetryPolicy::impatient(),
    )
    .unwrap()
    .messages_delta(&f.inbox, None, 1)
    .await
    .unwrap();

    assert!(format!("{:?}", first_page.items[0]).contains("Newest"));
    sync.sync_messages(&f.inbox).await.unwrap();
    let listed: Vec<String> =
        f.db.messages(&f.inbox, 0, 10)
            .unwrap()
            .into_iter()
            .map(|m| m.subject)
            .collect();
    assert_eq!(listed, ["Newest", "Middle", "Old"]);
}

#[tokio::test]
async fn every_folder_has_its_newest_mail_before_any_is_fetched_whole() {
    let f = Fixture::new().await;
    let sent = f.graph.folder_id("sentitems");
    f.graph.add_messages(&f.inbox, 6);
    f.graph
        .add_message(&sent, message("Old reply", "2026-01-01T09:00:00Z"));
    f.graph
        .add_message(&sent, message("New reply", "2026-10-01T09:00:00Z"));

    f.sync(1).sync_all().await.unwrap();

    // Which folder each page of messages was asked of, in the order asked.
    let asked: Vec<bool> = f
        .graph
        .requests()
        .iter()
        .filter(|request| request.contains("/messages/delta"))
        .map(|request| request.contains(&f.inbox))
        .collect();
    let first_of_sent = asked.iter().position(|inbox| !inbox).unwrap();
    let second_of_inbox = asked
        .iter()
        .enumerate()
        .filter(|(_, inbox)| **inbox)
        .nth(1)
        .unwrap()
        .0;
    assert!(
        first_of_sent < second_of_inbox,
        "Sent Items waited for the whole inbox: {asked:?}"
    );
    // And it all arrives in the end, as before.
    f.assert_in_step();

    // A sync after that has no first pages to fetch: one look at each folder.
    let before = f.graph.requests().len();
    f.sync(1).sync_all().await.unwrap();
    let again = f.graph.requests()[before..]
        .iter()
        .filter(|request| request.contains(&f.inbox) && request.contains("/messages/delta"))
        .count();
    assert_eq!(again, 1);
}

#[tokio::test]
async fn later_syncs_pick_up_every_kind_of_change() {
    let f = Fixture::new().await;
    let ids = f.graph.add_messages(&f.inbox, 40);
    let archive = f.graph.folder_id("archive");
    let sync = f.sync(7);
    sync.sync_all().await.unwrap();

    f.graph
        .add_message(&f.inbox, message("Brand new", "2026-10-09T10:00:00Z"));
    f.graph.set_read(&ids[0], true);
    f.graph.set_subject(&ids[1], "Renamed");
    f.graph.move_message(&ids[2], &archive);
    f.graph.delete_message(&ids[3]);
    let projects = f.graph.add_folder("Projects", None);
    f.graph.add_message(
        &projects,
        message("In a new folder", "2026-10-09T11:00:00Z"),
    );
    f.graph.rename_folder(&archive, "Old mail");
    let synced = sync.sync_all().await.unwrap();

    f.assert_in_step();
    assert!(synced.removed >= 2);
    assert_eq!(f.db.message(&ids[2]).unwrap().unwrap().folder_id, archive);
    assert_eq!(f.db.message(&ids[3]).unwrap(), None);

    // And with nothing changed, a sync changes nothing.
    assert_eq!(sync.sync_all().await.unwrap().changed, 0);
    f.assert_in_step();
}

#[tokio::test]
async fn a_moved_message_survives_whichever_folder_is_synced_first() {
    for destination_first in [true, false] {
        let f = Fixture::new().await;
        let id = f
            .graph
            .add_message(&f.inbox, message("On the move", "2026-10-01T09:00:00Z"));
        let archive = f.graph.folder_id("archive");
        let sync = f.sync(10);
        sync.sync_all().await.unwrap();

        f.graph.move_message(&id, &archive);
        let order = if destination_first {
            [&archive, &f.inbox]
        } else {
            [&f.inbox, &archive]
        };
        for folder in order {
            sync.sync_messages(folder).await.unwrap();
        }

        assert_eq!(
            f.db.message(&id).unwrap().unwrap().folder_id,
            archive,
            "destination first: {destination_first}"
        );
        sync.sync_all().await.unwrap();
        f.assert_in_step();
    }
}

#[tokio::test]
async fn an_interrupted_first_sync_carries_on_from_where_it_stopped() {
    let f = Fixture::new().await;
    f.graph.add_messages(&f.inbox, 100);
    f.sync(10).sync_folders().await.unwrap();

    // Three pages in, the connection goes and stays gone for this attempt.
    let before = f.graph.requests().len();
    let stopped = {
        let sync = f.sync(10);
        let run = sync.sync_messages(&f.inbox);
        tokio::pin!(run);
        loop {
            tokio::select! {
                biased;
                result = &mut run => break result.is_ok(),
                () = tokio::task::yield_now() => {
                    if f.graph.requests().len() >= before + 3 { break false; }
                }
            }
        }
    };
    assert!(!stopped, "the sync finished before it could be interrupted");
    let saved = f.db.message_count(&f.inbox).unwrap();
    assert!(
        (20..100).contains(&saved),
        "{saved} messages saved before the interruption"
    );
    let state = f.db.messages_sync(&f.inbox).unwrap();
    assert!(!state.complete && state.link.unwrap().contains("$skiptoken"));

    let resumed_at = f.graph.requests().len();
    f.sync(10).sync_messages(&f.inbox).await.unwrap();

    f.assert_in_step();
    // It asked only for the pages it did not already have.
    let asked = f.graph.requests().len() - resumed_at;
    assert!(
        asked <= 10 - (saved as usize / 10) + 1,
        "{asked} requests to fetch what was left after {saved}"
    );
}

#[tokio::test]
async fn throttling_and_outages_slow_a_sync_down_but_lose_nothing() {
    let f = Fixture::new().await;
    f.graph.add_messages(&f.inbox, 300);
    f.graph.fail_next([
        Fault::Throttled {
            retry_after_secs: 0,
        },
        Fault::Unavailable,
        Fault::Throttled {
            retry_after_secs: 0,
        },
    ]);
    // Then nearly a third of all requests fail, at random.
    f.graph.fail_at_random(300, 7);

    // A request that fails six times running gives up; the sync is simply run again.
    let sync = f.sync(10);
    let mut attempts = 0;
    loop {
        attempts += 1;
        match sync.sync_all().await {
            Ok(_) => break,
            Err(e) if e.is_transient() && attempts < 50 => {}
            Err(e) => panic!("after {attempts} attempts: {e}"),
        }
    }

    f.graph.stop_failing();
    f.assert_in_step();
    assert!(f.graph.requests().len() > 40, "nothing was retried");
}

#[tokio::test]
async fn being_throttled_for_good_is_reported_and_nothing_is_lost() {
    let f = Fixture::new().await;
    f.graph.add_messages(&f.inbox, 30);
    let sync = f.sync(10);
    sync.sync_all().await.unwrap();
    f.graph
        .add_message(&f.inbox, message("While throttled", "2026-10-09T10:00:00Z"));
    f.graph.fail_at_random(1000, 1);

    let error = sync.sync_all().await.unwrap_err();

    assert!(error.is_transient(), "{error:?}");
    assert_eq!(f.db.message_count(&f.inbox).unwrap(), 30);
    f.graph.stop_failing();
    sync.sync_all().await.unwrap();
    f.assert_in_step();
}

#[tokio::test]
async fn an_expired_delta_token_brings_a_fresh_pass_that_removes_only_what_is_gone() {
    let f = Fixture::new().await;
    let ids = f.graph.add_messages(&f.inbox, 60);
    let sync = f.sync(10);
    sync.sync_all().await.unwrap();

    // Changes made while the token was good but never fetched, then the token expires.
    f.graph.delete_message(&ids[5]);
    f.graph.set_read(&ids[6], true);
    f.graph
        .add_message(&f.inbox, message("After the reset", "2026-10-09T10:00:00Z"));
    let doomed = f.graph.add_folder("Doomed", None);
    f.graph.remove_folder(&doomed);
    f.graph.expire_delta_tokens();
    let synced = sync.sync_all().await.unwrap();

    f.assert_in_step();
    assert!(
        synced.restarts >= 2,
        "the folder list and the inbox should each have started again"
    );
    assert_eq!(f.db.message(&ids[5]).unwrap(), None);
    assert!(f.db.message(&ids[6]).unwrap().unwrap().is_read);
}

#[tokio::test]
async fn nothing_is_thrown_away_while_a_fresh_pass_is_under_way() {
    let f = Fixture::new().await;
    f.graph.add_messages(&f.inbox, 50);
    let sync = f.sync(10);
    sync.sync_all().await.unwrap();
    f.graph.expire_delta_tokens();
    f.graph.fail_next([]);

    // The fresh pass gets one page in and is cut off.
    let before = f.graph.requests().len();
    {
        let run = sync.sync_messages(&f.inbox);
        tokio::pin!(run);
        loop {
            tokio::select! {
                biased;
                _ = &mut run => break,
                () = tokio::task::yield_now() => if f.graph.requests().len() >= before + 3 { break },
            }
        }
    }

    assert_eq!(
        f.db.message_count(&f.inbox).unwrap(),
        50,
        "messages were dropped before the pass finished"
    );
    f.sync(10).sync_all().await.unwrap();
    f.assert_in_step();
}

#[tokio::test]
async fn tokens_expiring_in_the_middle_of_a_first_sync_start_it_again() {
    let f = Fixture::new().await;
    f.graph.add_messages(&f.inbox, 40);
    let sync = f.sync(10);
    sync.sync_folders().await.unwrap();
    // Read one page by hand, so a next link is saved, then expire it.
    let client = GraphClient::new(
        f.graph.base_url(),
        Arc::new(FixedToken),
        RetryPolicy::impatient(),
    )
    .unwrap();
    let page = client.messages_delta(&f.inbox, None, 10).await.unwrap();
    f.db.apply_messages(
        &f.inbox,
        &[],
        &[],
        page.next.url(),
        brolga_store::Round::Continues,
    )
    .unwrap();
    f.graph.expire_delta_tokens();

    let synced = sync.sync_all().await.unwrap();

    assert!(synced.restarts >= 1);
    f.assert_in_step();
}

#[tokio::test]
async fn a_server_that_never_lets_a_sync_finish_is_reported_not_looped_on() {
    let f = Fixture::new().await;
    f.graph.add_messages(&f.inbox, 15);
    let sync = f.sync(10);
    sync.sync_all().await.unwrap();
    f.graph.never_settle(true);

    let error = sync.sync_all().await.unwrap_err();

    assert!(matches!(error, SyncError::NeverSettled(_)), "{error:?}");
    assert!(!error.is_transient());
    // Everything already here is still here.
    assert_eq!(f.db.message_count(&f.inbox).unwrap(), 15);
    f.graph.never_settle(false);
    sync.sync_all().await.unwrap();
    f.assert_in_step();
}

#[tokio::test]
async fn a_folder_deleted_on_the_server_takes_its_messages_with_it() {
    let f = Fixture::new().await;
    let projects = f.graph.add_folder("Projects", None);
    f.graph.add_messages(&projects, 12);
    f.graph.add_messages(&f.inbox, 5);
    let sync = f.sync(10);
    sync.sync_all().await.unwrap();
    assert_eq!(f.db.message_count(&projects).unwrap(), 12);

    f.graph.remove_folder(&projects);
    sync.sync_all().await.unwrap();

    f.assert_in_step();
    assert_eq!(f.db.message_count(&projects).unwrap(), 0);
    assert_eq!(f.db.messages_sync(&projects).unwrap().link, None);
}

/// The chaos test: the mailbox keeps changing, a third of requests fail,
/// tokens expire, and the sync is cut off at arbitrary points and started
/// again as a new process would. At the end the two must agree exactly.
#[tokio::test]
async fn chaos_ends_with_the_database_matching_the_server() {
    for seed in 1..=4u64 {
        let f = Fixture::new().await;
        let archive = f.graph.folder_id("archive");
        let mut ids = f.graph.add_messages(&f.inbox, 120);
        let mut random = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let mut roll = move |sides: u64| {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            random % sides
        };
        f.graph.fail_at_random(250, seed);

        for step in 0..40 {
            // The mailbox changes under the sync.
            match roll(6) {
                0 => ids.push(f.graph.add_message(
                    &f.inbox,
                    message(&format!("New at step {step}"), "2026-10-09T10:00:00Z"),
                )),
                1 => f
                    .graph
                    .set_read(&ids[roll(ids.len() as u64) as usize], true),
                2 => f
                    .graph
                    .move_message(&ids[roll(ids.len() as u64) as usize], &archive),
                3 => f
                    .graph
                    .move_message(&ids[roll(ids.len() as u64) as usize], &f.inbox),
                4 => f
                    .graph
                    .delete_message(&ids[roll(ids.len() as u64) as usize]),
                _ => f.graph.set_subject(
                    &ids[roll(ids.len() as u64) as usize],
                    &format!("Edited at step {step}"),
                ),
            }
            if step % 13 == 12 {
                f.graph.expire_delta_tokens();
            }

            // A sync that may be killed at any await, as the process may be.
            let sync = f.sync(1 + roll(15) as u32);
            let _ = tokio::time::timeout(Duration::from_micros(200 + roll(4000)), sync.sync_all())
                .await;
        }

        f.graph.stop_failing();
        f.sync(25).sync_all().await.unwrap();
        f.assert_in_step();
    }
}

/// The milestone's acceptance run: a mailbox of 100,000 messages, synced
/// under throttling and interrupted part-way, then timed for the list and
/// search targets in the build spec. It takes a while, so it is run on
/// request: `cargo test --release -p brolga-sync -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "takes a minute; run with --release -- --ignored"]
async fn a_hundred_thousand_messages_sync_resumably_and_stay_fast() {
    use std::time::Instant;

    let folder = std::env::temp_dir().join(format!("brolga-100k-{}", std::process::id()));
    let graph = FakeGraph::start().await.unwrap();
    let inbox = graph.folder_id("inbox");
    graph.add_messages(&inbox, 100_000);
    graph.fail_at_random(50, 3);
    let open = || Arc::new(AccountDb::open(&folder.join("account.db")).unwrap());
    let sync = |db: Arc<AccountDb>| {
        let client = GraphClient::new(
            graph.base_url(),
            Arc::new(FixedToken),
            RetryPolicy::impatient(),
        )
        .unwrap();
        MailSync::new(Arc::new(client), db).with_page_size(100)
    };

    // Cut off three times on the way, each time starting again as a new process would.
    let started = Instant::now();
    for cut_after in [2, 5, 9] {
        let db = open();
        let _ = tokio::time::timeout(Duration::from_secs(cut_after), sync(db).sync_all()).await;
    }
    let db = open();
    let before_last = db.message_count(&inbox).unwrap();
    let mut attempts = 0;
    while let Err(e) = sync(db.clone()).sync_all().await {
        attempts += 1;
        assert!(e.is_transient() && attempts < 200, "{e}");
    }
    println!(
        "synced 100,000 messages in {:.1?}, {before_last} of them before the last restart",
        started.elapsed()
    );

    assert_eq!(db.message_count(&inbox).unwrap(), 100_000);
    assert!(
        before_last > 0 && before_last < 100_000,
        "the sync was never interrupted part-way: {before_last}"
    );
    let server = graph.messages();
    let local = db.messages(&inbox, 0, u32::MAX).unwrap();
    assert!(local.iter().all(|m| server.contains_key(&m.id)));

    // The targets, on a database opened cold.
    drop(db);
    let db = open();
    let time = |what: &str, limit_ms: u128, run: &dyn Fn() -> usize| {
        let started = Instant::now();
        let rows = run();
        let took = started.elapsed();
        println!("{what}: {rows} rows in {took:.2?} (target under {limit_ms} ms)");
        assert!(took.as_millis() < limit_ms, "{what} took {took:?}");
    };
    time("open a folder, first screen", 100, &|| {
        db.messages(&inbox, 0, 50).unwrap().len()
    });
    time("scroll to the middle", 100, &|| {
        db.messages(&inbox, 50_000, 50).unwrap().len()
    });
    time("scroll to the end", 100, &|| {
        db.messages(&inbox, 99_950, 50).unwrap().len()
    });
    time("sorted by sender, first screen", 100, &|| {
        db.messages_in_order(&inbox, Order::Sender, 0, 50)
            .unwrap()
            .len()
    });
    time("sorted by subject, the end", 100, &|| {
        db.messages_in_order(&inbox, Order::Subject, 99_950, 50)
            .unwrap()
            .len()
    });
    time("as conversations, first screen", 100, &|| {
        db.conversations(&inbox, 0, 50).unwrap().len()
            + db.conversation_count(&inbox).unwrap() as usize
    });
    time("as conversations, the end", 100, &|| {
        db.conversations(&inbox, 99_950, 50).unwrap().len()
    });
    time("suggest people as an address is typed", 100, &|| {
        db.suggest_people("sender4", 6).unwrap().len()
    });
    time("open a message", 100, &|| {
        usize::from(db.message(&local[70_000].id).unwrap().is_some())
    });
    time("search, a common word", 200, &|| {
        db.search("\"message\" *", None, None, None, None, None, 50)
            .unwrap()
            .len()
    });
    time("search, a rare word", 200, &|| {
        db.search("\"99999\" *", None, None, None, None, None, 50)
            .unwrap()
            .len()
    });
    time("search, by sender", 200, &|| {
        db.search("sender : \"sender42\" *", None, None, None, None, None, 50)
            .unwrap()
            .len()
    });
    time("search, unread with attachments", 200, &|| {
        db.search("", None, Some(false), Some(false), None, None, 50)
            .unwrap()
            .len()
    });

    drop(db);
    let _ = std::fs::remove_dir_all(folder);
}

#[tokio::test]
async fn a_question_put_to_the_server_comes_back_as_it_was_answered_taken_or_refused() {
    let f = Fixture::new().await;
    f.graph.add_messages(&f.inbox, 3);
    let client = GraphClient::new(
        f.graph.base_url(),
        Arc::new(FixedToken),
        RetryPolicy::impatient(),
    )
    .unwrap();

    let (status, said) = client
        .probe(&format!("/mailFolders/{}/messages/delta", f.inbox))
        .await
        .unwrap();
    assert_eq!(status, 200);
    assert!(said.contains("\"value\""), "{said}");

    // A refusal is an answer too, and is not tried again or made an error of.
    let before = f.graph.requests().len();
    let (status, said) = client
        .probe(&format!(
            "/mailFolders/{}/messages/delta?$expand=singleValueExtendedProperties($filter=id eq 'Integer 0x1081')",
            f.inbox
        ))
        .await
        .unwrap();
    assert_eq!(status, 400);
    assert!(
        said.contains("Parsing OData Select and Expand failed"),
        "{said}"
    );
    assert_eq!(f.graph.requests().len(), before + 1);

    assert!(client.probe("messages").await.is_err());
}

#[tokio::test]
async fn messages_answered_or_forwarded_elsewhere_are_marked_a_part_at_a_time_and_then_only_what_changed()
 {
    let f = Fixture::new().await;
    let ids = f.graph.add_messages(&f.inbox, 260);
    // All but the last ten were answered, in one way or another.
    for (n, id) in ids.iter().take(250).enumerate() {
        f.graph
            .set_last_verb_at(id, 102 + (n % 3) as u32, "2026-01-01T09:00:00Z");
    }
    let sync = f.sync(100);
    sync.sync_all().await.unwrap();
    let marked = || {
        f.db.messages(&f.inbox, 0, u32::MAX)
            .unwrap()
            .into_iter()
            .filter(|m| !m.last_action.is_empty())
            .count()
    };
    assert_eq!(marked(), 0, "a folder's sync does not say");

    // One page, and it stops: the rest is for the next sync.
    assert_eq!(sync.sync_last_actions(1).await.unwrap(), 200);
    assert_eq!(marked(), 200);
    // A freshly started Brolga carries on from where that one stopped.
    assert_eq!(f.sync(100).sync_last_actions(5).await.unwrap(), 50);
    assert_eq!(marked(), 250);
    let how = |id: &str| f.db.message(id).unwrap().unwrap().last_action;
    assert_eq!(
        (how(&ids[0]), how(&ids[1]), how(&ids[2]), how(&ids[259])),
        (
            "replied".to_owned(),
            "replied_all".to_owned(),
            "forwarded".to_owned(),
            String::new()
        )
    );

    // After a whole pass, only what has changed since is asked after.
    f.graph.set_last_verb(&ids[259], 104);
    let before = f.graph.requests().len();
    // One request, and the one message given: not the 250 again.
    assert_eq!(f.sync(100).sync_last_actions(5).await.unwrap(), 1);
    assert_eq!(f.graph.requests().len(), before + 1);
    assert_eq!(how(&ids[259]), "forwarded");
}

#[tokio::test]
async fn a_folder_the_server_will_not_give_does_not_stop_the_folders_after_it() {
    for status in [500, 403] {
        let f = Fixture::new().await;
        let (sent, archive) = (f.graph.folder_id("sentitems"), f.graph.folder_id("archive"));
        f.graph
            .add_message(&f.inbox, message("In", "2026-10-01T09:00:00Z"));
        f.graph
            .add_message(&sent, message("Sent", "2026-10-02T09:00:00Z"));
        f.graph
            .add_message(&archive, message("Kept", "2026-10-03T09:00:00Z"));
        // The inbox is synced first, and is the one that cannot be read.
        f.graph.break_folder(&f.inbox, status);

        let outcome = f.sync(10).sync_each().await;

        // Every other folder is as the server has it, and the trouble is said.
        assert_eq!(f.db.message_count(&sent).unwrap(), 1, "{status}");
        assert_eq!(f.db.message_count(&archive).unwrap(), 1, "{status}");
        assert_eq!(outcome.failed.len(), 1, "{:?}", outcome.failed);
        assert_eq!(outcome.failed[0].0, "Inbox");
        assert!(!outcome.stopped);
        assert!(f.sync(10).sync_all().await.is_err());

        // Once it can be read again, nothing was lost by the wait.
        f.graph.mend_folder(&f.inbox);
        f.sync(10).sync_all().await.unwrap();
        f.assert_in_step();
    }
}

#[tokio::test]
async fn an_account_that_cannot_be_reached_is_not_asked_folder_by_folder() {
    let f = Fixture::new().await;
    f.graph
        .add_message(&f.inbox, message("In", "2026-10-01T09:00:00Z"));
    f.sync(10).sync_all().await.unwrap();
    let before = f.graph.requests().len();

    // Throttled for good: every request is turned away.
    f.graph.fail_next(std::iter::repeat_n(
        Fault::Throttled {
            retry_after_secs: 0,
        },
        500,
    ));
    let outcome = f.sync(10).sync_each().await;
    f.graph.clear_faults();

    // One request, tried as often as a request is, and no more folders after it.
    assert!(outcome.stopped);
    assert_eq!(outcome.failed.len(), 1);
    assert_eq!(f.graph.requests().len() - before, 6);
}

#[tokio::test]
async fn a_message_with_a_part_that_cannot_be_read_is_kept_without_that_part() {
    let f = Fixture::new().await;
    let odd = f
        .graph
        .add_message(&f.inbox, message("Odd one", "2026-10-02T09:00:00Z"));
    let plain = f
        .graph
        .add_message(&f.inbox, message("Plain", "2026-10-01T09:00:00Z"));
    // A sender with no address at all, and a recipient list that is no list.
    f.graph.send_raw(
        &odd,
        serde_json::json!({ "from": { "emailAddress": null }, "toRecipients": "nobody", "isRead": "perhaps" }),
    );

    let outcome = f.sync(10).sync_each().await;

    // Both are here: the odd one with what could be read of it.
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    assert_eq!(outcome.synced.damaged, 1);
    let kept = f.db.message(&odd).unwrap().unwrap();
    assert_eq!(kept.subject, "Odd one");
    assert_eq!(kept.from_address, "");
    assert_eq!(f.db.message(&plain).unwrap().unwrap().subject, "Plain");
}

#[tokio::test]
async fn a_body_with_nothing_in_it_is_an_empty_body_and_the_ones_after_it_are_fetched() {
    let f = Fixture::new().await;
    let empty = f
        .graph
        .add_message(&f.inbox, message("Empty", "2026-10-02T09:00:00Z"));
    let full = f.graph.add_message(
        &f.inbox,
        message("Full", "2026-10-01T09:00:00Z").body("<p>Words</p>"),
    );
    f.graph.send_raw(
        &empty,
        serde_json::json!({ "body": { "contentType": "html", "content": null } }),
    );
    let sync = f.sync(10);
    sync.sync_all().await.unwrap();

    let fetched = sync.download_bodies("2026-01-01", 10).await.unwrap();

    assert_eq!(fetched, 2);
    assert_eq!(f.db.body(&empty).unwrap().unwrap().content, "");
    assert!(f.db.body(&full).unwrap().unwrap().content.contains("Words"));
}

#[tokio::test]
async fn a_pass_over_what_was_answered_keeps_its_place_through_an_outage() {
    let f = Fixture::new().await;
    let ids = f.graph.add_messages(&f.inbox, 260);
    for id in ids.iter().take(250) {
        f.graph.set_last_verb_at(id, 102, "2026-01-01T09:00:00Z");
    }
    let sync = f.sync(100);
    sync.sync_all().await.unwrap();
    assert_eq!(sync.sync_last_actions(1).await.unwrap(), 200);

    // The server cannot be had for a while. That is not the saved place's fault.
    f.graph.fail_next(std::iter::repeat_n(
        Fault::Throttled {
            retry_after_secs: 0,
        },
        6,
    ));
    assert!(sync.sync_last_actions(5).await.unwrap_err().is_transient());

    // The pass carries on from where it was: one request, for the last fifty.
    let before = f.graph.requests().len();
    assert_eq!(f.sync(100).sync_last_actions(5).await.unwrap(), 50);
    assert_eq!(f.graph.requests().len(), before + 1);
}

#[tokio::test]
async fn a_place_the_server_no_longer_takes_is_dropped_and_the_pass_begins_again() {
    let f = Fixture::new().await;
    let ids = f.graph.add_messages(&f.inbox, 30);
    for id in ids.iter().take(20) {
        f.graph.set_last_verb_at(id, 104, "2026-01-01T09:00:00Z");
    }
    let sync = f.sync(100);
    sync.sync_all().await.unwrap();
    f.db.set_setting(
        "last_actions.link",
        &format!(
            "{}/me/messages?$top=200&$skiptoken=stale",
            f.graph.base_url()
        ),
    )
    .unwrap();

    // Refused: said once, and the next pass starts from the beginning.
    assert!(matches!(
        sync.sync_last_actions(5).await,
        Err(SyncError::Graph(GraphError::Refused { status: 400, .. }))
    ));
    assert_eq!(sync.sync_last_actions(5).await.unwrap(), 20);
}

/// The day a message's flag is due, as a sync made in this zone keeps it.
async fn due_as_read(f: &Fixture, zone: &str, id: &str) -> String {
    let zone = jiff::tz::TimeZone::get(zone).unwrap();
    // Every message is read afresh, as a first sync reads it.
    f.db.restart_messages_sync(&f.inbox).unwrap();
    f.sync(10).with_zone(zone).sync_all().await.unwrap();

    f.db.message(id).unwrap().unwrap().flag_due
}

#[tokio::test]
async fn a_flag_is_due_on_the_day_it_is_due_where_the_reader_is() {
    let f = Fixture::new().await;
    let due = |date_time: &str, zone: &str| {
        let id = f
            .graph
            .add_message(&f.inbox, message("Follow up", "2026-10-01T09:00:00Z"));
        f.graph.send_raw(
            &id,
            serde_json::json!({ "flag": {
                "flagStatus": "flagged",
                "dueDateTime": { "dateTime": date_time, "timeZone": zone }
            } }),
        );
        id
    };
    // Due on the 21st, set where it is ten hours ahead of UTC: the first
    // moment of that day there is the afternoon of the 20th in UTC.
    let ahead = due("2026-10-20T14:00:00.0000000", "UTC");
    // The same day, given in the zone it was set in, by its Windows name.
    let named = due("2026-10-21T00:00:00.0000000", "E. Australia Standard Time");
    // And as an older Brolga wrote it: the day, at midnight UTC.
    let older = due("2026-10-21T00:00:00.0000000", "UTC");
    assert_eq!(
        due_as_read(&f, "Australia/Brisbane", &ahead).await,
        "2026-10-21"
    );
    assert_eq!(
        due_as_read(&f, "Australia/Brisbane", &named).await,
        "2026-10-21"
    );
    assert_eq!(
        due_as_read(&f, "Australia/Brisbane", &older).await,
        "2026-10-21"
    );
    // Behind UTC, the first moment of the 21st is four hours into it in UTC.
    let behind = due("2026-10-21T04:00:00.0000000", "UTC");
    assert_eq!(
        due_as_read(&f, "America/New_York", &behind).await,
        "2026-10-21"
    );
    assert_eq!(
        due_as_read(&f, "America/New_York", &older).await,
        "2026-10-21"
    );
}
