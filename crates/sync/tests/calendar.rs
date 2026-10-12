#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Calendar sync against the fake Graph server. What is kept on this
//! computer is held against what the server truly has, after every sync.

use std::sync::Arc;

use brolga_calendar::{DayOfWeek, Ends, Pattern, Series};
use brolga_fake_graph::{FakeEvent, FakeGraph, Fault};
use brolga_graph::{GraphClient, GraphError, RetryPolicy, TokenSource};
use brolga_store::AccountDb;
use brolga_sync::CalendarSync;
use futures::future::BoxFuture;
use jiff::Timestamp;
use jiff::civil::date;

struct Token(&'static str);

impl TokenSource for Token {
    fn token(&self) -> BoxFuture<'_, Result<String, GraphError>> {
        Box::pin(async { Ok(self.0.to_owned()) })
    }
}

struct Fixture {
    graph: FakeGraph,
    db: Arc<AccountDb>,
    sync: CalendarSync,
    main: String,
}

/// The moment the tests take to be now: the middle of October 2026.
fn now() -> Timestamp {
    "2026-10-15T00:00:00Z".parse().unwrap()
}

async fn fixture_with(token: &'static str) -> Fixture {
    let graph = FakeGraph::start().await.unwrap();
    let db = Arc::new(AccountDb::in_memory().unwrap());
    let client = Arc::new(
        GraphClient::new(
            graph.base_url(),
            Arc::new(Token(token)),
            RetryPolicy::impatient(),
        )
        .unwrap(),
    );

    Fixture {
        sync: CalendarSync::new(client, db.clone()).with_page_size(7),
        main: graph.default_calendar(),
        graph,
        db,
    }
}

async fn fixture() -> Fixture {
    fixture_with("token.Calendars.Read").await
}

const ALL: (&str, &str) = ("2000-01-01T00:00:00Z", "2100-01-01T00:00:00Z");
/// The window the client keeps, with now as it is in these tests.
const KEPT: (&str, &str) = ("2025-10-15T00:00:00Z", "2028-10-14T00:00:00Z");

impl Fixture {
    /// Every event here, as (id, start, end), to hold against the server's.
    fn here(&self, calendar: &str) -> Vec<(String, String, String)> {
        let mut all: Vec<(String, String, String)> = self
            .db
            .events_between(ALL.0, ALL.1, false)
            .unwrap()
            .into_iter()
            .filter(|e| e.calendar_id == calendar)
            .map(|e| (e.id, e.start, e.end))
            .collect();
        all.sort();
        all
    }

    /// What is here is exactly what the server has in the window.
    fn assert_in_step(&self, calendar: &str) {
        assert_eq!(
            self.here(calendar),
            self.graph.events(calendar, KEPT.0, KEPT.1)
        );
    }
}

fn weekly_monday_nine(zone: &str, from: jiff::civil::Date) -> Series {
    Series {
        start: from.at(9, 0, 0, 0),
        end: from.at(10, 0, 0, 0),
        zone: zone.to_owned(),
        pattern: Pattern::Weekly {
            interval: 1,
            days_of_week: vec![DayOfWeek::Monday],
            first_day_of_week: DayOfWeek::Sunday,
        },
        from,
        ends: Ends::Never,
    }
}

#[tokio::test]
async fn a_first_sync_brings_down_the_calendars_and_their_events() {
    let f = fixture().await;
    let work = f.graph.add_calendar("Work", "#2f6fce");
    f.graph.add_event(
        &f.main,
        FakeEvent::new("Dentist", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z")
            .at("Harbour Dental"),
    );
    f.graph.add_event(
        &work,
        FakeEvent::new("Stand-up", "2026-10-20T23:00:00Z", "2026-10-20T23:15:00Z"),
    );

    f.sync.sync_all(now()).await.unwrap();

    let calendars = f.db.calendars().unwrap();
    assert_eq!(calendars.len(), 2);
    assert!(calendars[0].is_default);
    assert_eq!(
        (calendars[1].name.as_str(), calendars[1].hex_color.as_str()),
        ("Work", "#2f6fce")
    );
    f.assert_in_step(&f.main);
    f.assert_in_step(&work);
    let dentist = &f
        .db
        .events_between("2026-10-20T00:00:00Z", "2026-10-20T03:00:00Z", true)
        .unwrap()[0];
    assert_eq!(
        (dentist.subject.as_str(), dentist.location.as_str()),
        ("Dentist", "Harbour Dental")
    );
    assert_eq!(dentist.reminder_minutes, Some(15));
    assert_eq!(dentist.start_zone, "E. Australia Standard Time");
}

#[tokio::test]
async fn each_occurrence_of_a_repeating_event_is_kept_at_the_moment_the_server_gives_it() {
    let f = fixture().await;
    // Weekly at nine in Sydney, across the start of daylight saving on 4 October 2026.
    f.graph.add_event(
        &f.main,
        FakeEvent::repeating(
            "Planning",
            weekly_monday_nine("Australia/Sydney", date(2026, 9, 21)),
            "AUS Eastern Standard Time",
        ),
    );

    f.sync.sync_all(now()).await.unwrap();

    f.assert_in_step(&f.main);
    let starts: Vec<String> =
        f.db.events_between("2026-09-20T00:00:00Z", "2026-10-13T00:00:00Z", true)
            .unwrap()
            .into_iter()
            .map(|e| e.start)
            .collect();
    assert_eq!(
        starts,
        [
            "2026-09-20T23:00:00Z",
            "2026-09-27T23:00:00Z",
            "2026-10-04T22:00:00Z",
            "2026-10-11T22:00:00Z"
        ]
    );
    // Only what falls in the window kept is here, of a series with no end.
    let count = f.here(&f.main).len();
    assert!((100..110).contains(&count), "{count} occurrences");
    assert!(
        f.db.events_between(ALL.0, ALL.1, true)
            .unwrap()
            .iter()
            .all(|e| e.kind == "occurrence")
    );
}

#[tokio::test]
async fn a_moved_occurrence_and_a_cancelled_one_are_kept_as_the_server_has_them() {
    let f = fixture().await;
    f.graph.add_event(
        &f.main,
        FakeEvent::repeating(
            "Planning",
            weekly_monday_nine("Australia/Brisbane", date(2026, 10, 5)),
            "E. Australia Standard Time",
        )
        .cancel_on("2026-10-12")
        .move_on("2026-10-19", "2026-10-19T04:00:00Z", "2026-10-19T05:00:00Z"),
    );

    f.sync.sync_all(now()).await.unwrap();

    f.assert_in_step(&f.main);
    let october: Vec<(String, String)> =
        f.db.events_between("2026-10-04T00:00:00Z", "2026-10-27T00:00:00Z", true)
            .unwrap()
            .into_iter()
            .map(|e| (e.start, e.kind))
            .collect();
    assert_eq!(
        october,
        [
            ("2026-10-04T23:00:00Z".to_owned(), "occurrence".to_owned()),
            // The 12th is cancelled; the 19th was moved to two in the afternoon.
            ("2026-10-19T04:00:00Z".to_owned(), "exception".to_owned()),
            ("2026-10-25T23:00:00Z".to_owned(), "occurrence".to_owned()),
        ]
    );
}

#[tokio::test]
async fn later_syncs_bring_only_what_changed_and_end_in_step() {
    let f = fixture().await;
    let dentist = f.graph.add_event(
        &f.main,
        FakeEvent::new("Dentist", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z"),
    );
    let lunch = f.graph.add_event(
        &f.main,
        FakeEvent::new("Lunch", "2026-10-21T02:00:00Z", "2026-10-21T03:00:00Z"),
    );
    let series = f.graph.add_event(
        &f.main,
        FakeEvent::repeating(
            "Planning",
            weekly_monday_nine("Australia/Brisbane", date(2026, 10, 5)),
            "E. Australia Standard Time",
        ),
    );
    f.sync.sync_all(now()).await.unwrap();
    f.assert_in_step(&f.main);

    // Nothing has changed: nothing is fetched but the empty round.
    let idle = f.sync.sync_all(now()).await.unwrap();
    assert_eq!(
        (idle.changed, idle.removed),
        (1, 0),
        "only the list of calendars"
    );

    f.graph
        .change_event(&dentist, |e| e.start = "2026-10-20T03:00:00Z".to_owned());
    f.graph
        .change_event(&dentist, |e| e.end = "2026-10-20T04:00:00Z".to_owned());
    f.graph.delete_event(&lunch);
    f.graph.add_event(
        &f.main,
        FakeEvent::new("New", "2026-10-22T02:00:00Z", "2026-10-22T03:00:00Z"),
    );
    // One occurrence of the series is cancelled: it alone must go.
    f.graph
        .change_event(&series, |e| e.cancelled_days.push("2026-10-19".to_owned()));
    let before = f.here(&f.main).len();

    f.sync.sync_all(now()).await.unwrap();

    f.assert_in_step(&f.main);
    assert_eq!(
        f.here(&f.main).len(),
        before - 1,
        "lunch and one occurrence gone, one event new"
    );
    assert_eq!(
        f.db.event(&dentist).unwrap().unwrap().start,
        "2026-10-20T03:00:00Z"
    );

    // The whole series is deleted: every occurrence of it goes.
    f.graph.delete_event(&series);
    f.sync.sync_all(now()).await.unwrap();
    f.assert_in_step(&f.main);
    assert_eq!(f.here(&f.main).len(), 2);
}

#[tokio::test]
async fn when_the_server_asks_for_a_fresh_start_nothing_is_lost_and_what_went_meanwhile_goes() {
    let f = fixture().await;
    let stays = f.graph.add_event(
        &f.main,
        FakeEvent::new("Stays", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z"),
    );
    let goes = f.graph.add_event(
        &f.main,
        FakeEvent::new("Goes", "2026-10-21T01:00:00Z", "2026-10-21T02:00:00Z"),
    );
    f.sync.sync_all(now()).await.unwrap();

    f.graph.expire_calendar_tokens();
    f.graph.delete_event(&goes);
    let synced = f.sync.sync_all(now()).await.unwrap();

    assert_eq!(synced.restarts, 1);
    f.assert_in_step(&f.main);
    assert!(f.db.event(&stays).unwrap().is_some());
    assert!(f.db.event(&goes).unwrap().is_none());
}

#[tokio::test]
async fn a_sync_cut_short_carries_on_and_ends_in_step() {
    let f = fixture().await;
    f.graph.add_event(
        &f.main,
        FakeEvent::repeating(
            "Planning",
            weekly_monday_nine("Australia/Sydney", date(2026, 1, 5)),
            "AUS Eastern Standard Time",
        ),
    );
    let work = f.graph.add_calendar("Work", "");
    for day in 1..=28 {
        f.graph.add_event(
            &work,
            FakeEvent::new(
                "Shift",
                &format!("2026-11-{day:02}T01:00:00Z"),
                &format!("2026-11-{day:02}T09:00:00Z"),
            ),
        );
    }

    // The server falls over a few pages in, again and again.
    let mut attempts = 0;
    loop {
        f.graph
            .fail_after(3, std::iter::repeat_n(Fault::Unavailable, 12));
        attempts += 1;
        assert!(attempts < 60, "the sync never finished");
        if f.sync.sync_all(now()).await.is_ok() {
            break;
        }
    }
    f.graph.clear_faults();
    f.sync.sync_all(now()).await.unwrap();

    assert!(attempts > 1, "the sync was never cut short");
    f.assert_in_step(&f.main);
    f.assert_in_step(&work);
}

#[tokio::test]
async fn a_calendar_with_no_delta_is_read_whole_and_what_it_no_longer_has_goes() {
    let f = fixture().await;
    let work = f.graph.add_calendar("Work", "");
    let old = f.graph.add_event(
        &work,
        FakeEvent::new("Old", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z"),
    );
    f.sync.sync_all(now()).await.unwrap();

    f.graph.delete_event(&old);
    f.graph.add_event(
        &work,
        FakeEvent::new("New", "2026-10-21T01:00:00Z", "2026-10-21T02:00:00Z"),
    );

    // Straight away it is not read again: it has no delta, and is read on a slower turn.
    f.sync.sync_all(now()).await.unwrap();
    assert!(f.db.event(&old).unwrap().is_some());

    // When its turn comes round, it is. The turn is brought on here by
    // saying it is due, there being no waiting a quarter of an hour in a test.
    f.db.restart_events_sync(&work).unwrap();
    f.sync.sync_all(now()).await.unwrap();
    f.assert_in_step(&work);
    assert!(f.db.event(&old).unwrap().is_none());
}

#[tokio::test]
async fn a_calendar_deleted_on_the_server_goes_with_its_events() {
    let f = fixture().await;
    let work = f.graph.add_calendar("Work", "");
    f.graph.add_event(
        &work,
        FakeEvent::new("Shift", "2026-10-20T01:00:00Z", "2026-10-20T09:00:00Z"),
    );
    f.graph.add_event(
        &f.main,
        FakeEvent::new("Dentist", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z"),
    );
    f.sync.sync_all(now()).await.unwrap();
    assert_eq!(f.db.event_count().unwrap(), 2);

    f.graph.remove_calendar(&work);
    f.sync.sync_all(now()).await.unwrap();

    assert_eq!(f.db.calendars().unwrap().len(), 1);
    assert_eq!(f.db.event_count().unwrap(), 1);
}

#[tokio::test]
async fn an_all_day_event_is_kept_as_whole_days() {
    let f = fixture().await;
    f.graph.add_event(
        &f.main,
        FakeEvent::new("Conference", "2026-10-20T00:00:00Z", "2026-10-22T00:00:00Z").all_day(),
    );

    f.sync.sync_all(now()).await.unwrap();

    let event = &f.db.events_between(ALL.0, ALL.1, true).unwrap()[0];
    assert!(event.is_all_day);
    assert_eq!(
        (event.start.as_str(), event.end.as_str()),
        ("2026-10-20T00:00:00Z", "2026-10-22T00:00:00Z")
    );
}

#[tokio::test]
async fn a_sign_in_that_may_not_read_calendars_reads_none() {
    let f = fixture_with("token.Mail.ReadWrite").await;
    f.graph.add_event(
        &f.main,
        FakeEvent::new("Dentist", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z"),
    );

    assert!(f.sync.sync_all(now()).await.is_err());
    assert_eq!(f.db.event_count().unwrap(), 0);
}

#[tokio::test]
async fn a_calendar_the_server_will_not_give_does_not_stop_the_calendars_after_it() {
    let f = fixture().await;
    let work = f.graph.add_calendar("Work", "#2f6fce");
    let home = f.graph.add_calendar("Home", "#2f6fce");
    f.graph.add_event(
        &f.main,
        FakeEvent::new("Dentist", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z"),
    );
    f.graph.add_event(
        &work,
        FakeEvent::new("Shift", "2026-10-21T01:00:00Z", "2026-10-21T02:00:00Z"),
    );
    f.graph.add_event(
        &home,
        FakeEvent::new("Dinner", "2026-10-22T08:00:00Z", "2026-10-22T09:00:00Z"),
    );
    // The default calendar is read first, and then Work.
    f.graph.break_calendar(&f.main, 500);
    f.graph.break_calendar(&work, 403);

    let outcome = f.sync.sync_each(now()).await;

    assert_eq!(outcome.failed.len(), 2, "{:?}", outcome.failed);
    assert_eq!(outcome.failed[1].0, "Work");
    assert!(!outcome.stopped);
    f.assert_in_step(&home);
    assert!(f.sync.sync_all(now()).await.is_err());

    f.graph.mend_calendar(&f.main);
    f.graph.mend_calendar(&work);
    f.sync.sync_all(now()).await.unwrap();
    for calendar in [&f.main, &work, &home] {
        f.assert_in_step(calendar);
    }
}
