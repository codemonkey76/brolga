//! One account's calendars and events.
//!
//! An event here is what a calendar view gives: each occurrence of a
//! repeating event is a row of its own, already placed in time by the
//! server. Every time is kept in UTC, as `2026-10-20T03:00:00Z`, beside the
//! zone the event was made in, so the same text sorts and compares as time.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::account::{Round, SyncState, advance, restart, sync_state};
use crate::{AccountDb, StoreError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarRow {
    pub id: String,
    pub name: String,
    /// A named colour theme, such as `lightBlue`, or `auto`.
    pub color: String,
    /// `#RRGGBB`, or empty where the calendar has no colour of its own.
    pub hex_color: String,
    pub is_default: bool,
    pub can_edit: bool,
    pub owner_name: String,
    pub owner_address: String,
    /// Whether its events are shown. Kept on this computer only.
    pub shown: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EventRow {
    pub id: String,
    pub calendar_id: String,
    pub subject: String,
    pub preview: String,
    pub location: String,
    /// UTC: `2026-10-20T03:00:00Z`. For an all-day event, midnight of its first day.
    pub start: String,
    /// UTC. For an all-day event, midnight after its last day.
    pub end: String,
    /// It takes whole days, wherever it is seen from: its start and end
    /// are days, not moments.
    pub is_all_day: bool,
    /// The zone the event was made in, as Graph named it.
    pub start_zone: String,
    pub end_zone: String,
    /// `free`, `tentative`, `busy`, `oof`, `workingElsewhere` or `unknown`.
    pub show_as: String,
    /// `normal`, `personal`, `private` or `confidential`.
    pub sensitivity: String,
    pub is_cancelled: bool,
    pub is_organizer: bool,
    pub organizer_name: String,
    pub organizer_address: String,
    /// The owner's own answer: `none`, `organizer`, `tentativelyAccepted`,
    /// `accepted`, `declined` or `notResponded`.
    pub response: String,
    /// `singleInstance`, `occurrence`, `exception` or `seriesMaster`.
    pub kind: String,
    pub series_master_id: Option<String>,
    /// JSON: an array of names.
    pub categories: String,
    /// How many minutes before it starts to remind, where a reminder is set.
    pub reminder_minutes: Option<i64>,
    pub join_url: String,
    pub web_link: String,
    pub change_key: String,
    /// JSON: an array of `{name, address, kind, response}`.
    pub attendees: String,
}

const EVENT_COLUMNS: &str = "id, calendar_id, subject, preview, location, start_at, end_at, is_all_day, start_zone, \
                             end_zone, show_as, sensitivity, is_cancelled, is_organizer, organizer_name, \
                             organizer_address, response, kind, series_master_id, categories, reminder_minutes, \
                             join_url, web_link, change_key, attendees";

fn read_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRow> {
    Ok(EventRow {
        id: row.get(0)?,
        calendar_id: row.get(1)?,
        subject: row.get(2)?,
        preview: row.get(3)?,
        location: row.get(4)?,
        start: row.get(5)?,
        end: row.get(6)?,
        is_all_day: row.get(7)?,
        start_zone: row.get(8)?,
        end_zone: row.get(9)?,
        show_as: row.get(10)?,
        sensitivity: row.get(11)?,
        is_cancelled: row.get(12)?,
        is_organizer: row.get(13)?,
        organizer_name: row.get(14)?,
        organizer_address: row.get(15)?,
        response: row.get(16)?,
        kind: row.get(17)?,
        series_master_id: row.get(18)?,
        categories: row.get(19)?,
        reminder_minutes: row.get(20)?,
        join_url: row.get(21)?,
        web_link: row.get(22)?,
        change_key: row.get(23)?,
        attendees: row.get(24)?,
    })
}

fn events_resource(calendar_id: &str) -> String {
    format!("events:{calendar_id}")
}

fn insert_event(
    transaction: &rusqlite::Transaction<'_>,
    calendar_id: &str,
    event: &EventRow,
    generation: i64,
) -> Result<(), StoreError> {
    transaction.execute(
        &format!(
            "INSERT OR REPLACE INTO events ({EVENT_COLUMNS}, generation)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                     ?20, ?21, ?22, ?23, ?24, ?25, ?26)"
        ),
        params![
            event.id,
            calendar_id,
            event.subject,
            event.preview,
            event.location,
            event.start,
            event.end,
            event.is_all_day,
            event.start_zone,
            event.end_zone,
            event.show_as,
            event.sensitivity,
            event.is_cancelled,
            event.is_organizer,
            event.organizer_name,
            event.organizer_address,
            event.response,
            event.kind,
            event.series_master_id,
            event.categories,
            event.reminder_minutes,
            event.join_url,
            event.web_link,
            event.change_key,
            event.attendees,
            generation
        ],
    )?;

    Ok(())
}

impl AccountDb {
    /// Make the calendars here the ones the server has. A calendar it no
    /// longer has goes, with its events. Whether each is shown is this
    /// computer's own business, and is kept.
    pub fn set_calendars(&self, calendars: &[CalendarRow]) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        transaction.execute("UPDATE calendars SET seen = 0", [])?;
        for (position, calendar) in (0i64..).zip(calendars) {
            transaction.execute(
                "INSERT INTO calendars (id, name, color, hex_color, is_default, can_edit, owner_name, owner_address, position, seen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1)
                 ON CONFLICT (id) DO UPDATE SET
                     name = ?2, color = ?3, hex_color = ?4, is_default = ?5, can_edit = ?6,
                     owner_name = ?7, owner_address = ?8, position = ?9, seen = 1",
                params![
                    calendar.id,
                    calendar.name,
                    calendar.color,
                    calendar.hex_color,
                    calendar.is_default,
                    calendar.can_edit,
                    calendar.owner_name,
                    calendar.owner_address,
                    position
                ],
            )?;
        }
        transaction.execute(
            "DELETE FROM events WHERE calendar_id IN (SELECT id FROM calendars WHERE seen = 0)",
            [],
        )?;
        transaction.execute(
            "DELETE FROM sync_state WHERE resource IN (SELECT 'events:' || id FROM calendars WHERE seen = 0)",
            [],
        )?;
        transaction.execute("DELETE FROM calendars WHERE seen = 0", [])?;
        transaction.commit()?;

        Ok(())
    }

    /// Every calendar, the default one first, then as the server lists them.
    pub fn calendars(&self) -> Result<Vec<CalendarRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT id, name, color, hex_color, is_default, can_edit, owner_name, owner_address, shown
             FROM calendars ORDER BY is_default DESC, position, id",
        )?;
        let calendars = statement
            .query_map([], |row| {
                Ok(CalendarRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                    hex_color: row.get(3)?,
                    is_default: row.get(4)?,
                    can_edit: row.get(5)?,
                    owner_name: row.get(6)?,
                    owner_address: row.get(7)?,
                    shown: row.get(8)?,
                })
            })?
            .collect::<Result<_, _>>()?;

        Ok(calendars)
    }

    /// Show or hide a calendar's events. False if there is no such calendar.
    pub fn set_calendar_shown(&self, id: &str, shown: bool) -> Result<bool, StoreError> {
        let changed = self.connection().execute(
            "UPDATE calendars SET shown = ?2 WHERE id = ?1",
            params![id, shown],
        )?;

        Ok(changed > 0)
    }

    pub fn events_sync(&self, calendar_id: &str) -> Result<SyncState, StoreError> {
        sync_state(&self.connection(), &events_resource(calendar_id))
    }

    /// Read a calendar's events again from the start, keeping what is here
    /// until that pass is done.
    pub fn restart_events_sync(&self, calendar_id: &str) -> Result<(), StoreError> {
        restart(&self.connection(), &events_resource(calendar_id))
    }

    /// When a calendar's events were last brought up to date, in UTC.
    pub fn events_synced_at(&self, calendar_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                "SELECT synced_at FROM sync_state WHERE resource = ?1",
                [events_resource(calendar_id)],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Apply one page of a calendar's events, and where it leaves the
    /// sync, together or not at all. An event is given whole, so it
    /// replaces what was here for it.
    pub fn apply_events(
        &self,
        calendar_id: &str,
        changed: &[EventRow],
        removed: &[String],
        link: &str,
        round: Round,
    ) -> Result<(), StoreError> {
        let resource = events_resource(calendar_id);
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let state = sync_state(&transaction, &resource)?;

        for event in changed {
            insert_event(&transaction, calendar_id, event, state.generation)?;
        }
        for id in removed {
            // A view names as removed events that were never in it, as well
            // as ones deleted from it. Either way, if it is here it goes.
            transaction.execute(
                "DELETE FROM events WHERE id = ?1 AND calendar_id = ?2",
                params![id, calendar_id],
            )?;
        }
        if round == Round::Finished && state.reconciling {
            transaction.execute(
                "DELETE FROM events WHERE calendar_id = ?1 AND generation < ?2",
                params![calendar_id, state.generation],
            )?;
        }

        advance(&transaction, &resource, link, round)?;
        transaction.commit()?;

        Ok(())
    }

    /// Put in one event as the server has just given it, after it was made
    /// or changed from here. The sync is left where it was.
    pub fn put_event(&self, calendar_id: &str, event: &EventRow) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        let state = sync_state(&transaction, &events_resource(calendar_id))?;
        insert_event(&transaction, calendar_id, event, state.generation)?;
        transaction.commit()?;

        Ok(())
    }

    /// The owner's own answer to an invitation, as just given. The next
    /// sync replaces it with what the server says.
    pub fn set_event_response(&self, id: &str, response: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE events SET response = ?2 WHERE id = ?1",
            params![id, response],
        )?;

        Ok(())
    }

    /// Take an event out of this computer's copy: it was declined, and the
    /// server takes a declined event out of the calendar.
    pub fn forget_event(&self, id: &str) -> Result<(), StoreError> {
        self.connection()
            .execute("DELETE FROM events WHERE id = ?1", [id])?;

        Ok(())
    }

    /// The events that overlap the time from `after` up to `before`, both
    /// in UTC, earliest first. With `only_shown`, those of the calendars
    /// that are shown.
    pub fn events_between(
        &self,
        after: &str,
        before: &str,
        only_shown: bool,
    ) -> Result<Vec<EventRow>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(&format!(
            "SELECT {EVENT_COLUMNS} FROM events
             WHERE start_at < ?2 AND end_at > ?1 AND kind <> 'seriesMaster'
               AND (?3 = 0 OR calendar_id IN (SELECT id FROM calendars WHERE shown = 1))
             ORDER BY start_at, end_at, id"
        ))?;
        let events = statement
            .query_map(params![after, before, only_shown], read_event)?
            .collect::<Result<_, _>>()?;

        Ok(events)
    }

    pub fn event(&self, id: &str) -> Result<Option<EventRow>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                &format!("SELECT {EVENT_COLUMNS} FROM events WHERE id = ?1"),
                [id],
                read_event,
            )
            .optional()?)
    }

    /// The events with a reminder set that start between two moments in
    /// UTC, on calendars that are shown, each with what has been done about
    /// its reminder here: whether it was dismissed, and until when it was
    /// put off. Cancelled events remind of nothing.
    pub fn events_with_reminders(
        &self,
        after: &str,
        before: &str,
    ) -> Result<Vec<(EventRow, bool, Option<String>)>, StoreError> {
        let connection = self.connection();
        let columns = EVENT_COLUMNS
            .split(", ")
            .map(|column| format!("e.{}", column.trim()))
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = connection.prepare(&format!(
            "SELECT {columns}, coalesce(r.dismissed, 0), r.snoozed_until
             FROM events e LEFT JOIN reminders r ON r.event_id = e.id
             WHERE e.start_at >= ?1 AND e.start_at < ?2 AND e.reminder_minutes IS NOT NULL
               AND e.is_cancelled = 0 AND e.kind <> 'seriesMaster'
               AND e.calendar_id IN (SELECT id FROM calendars WHERE shown = 1)
             ORDER BY e.start_at, e.id"
        ))?;
        let found = statement
            .query_map(params![after, before], |row| {
                Ok((read_event(row)?, row.get(25)?, row.get(26)?))
            })?
            .collect::<Result<_, _>>()?;

        Ok(found)
    }

    /// Have done with an event's reminder.
    pub fn dismiss_reminder(&self, event_id: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO reminders (event_id, dismissed) VALUES (?1, 1)
             ON CONFLICT (event_id) DO UPDATE SET dismissed = 1",
            [event_id],
        )?;

        Ok(())
    }

    /// Put an event's reminder off until a moment in UTC.
    pub fn snooze_reminder(&self, event_id: &str, until: &str) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO reminders (event_id, snoozed_until) VALUES (?1, ?2)
             ON CONFLICT (event_id) DO UPDATE SET snoozed_until = ?2, dismissed = 0",
            params![event_id, until],
        )?;

        Ok(())
    }

    pub fn event_count(&self) -> Result<u32, StoreError> {
        Ok(self
            .connection()
            .query_row("SELECT count(*) FROM events", [], |row| row.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar(id: &str, is_default: bool) -> CalendarRow {
        CalendarRow {
            id: id.to_owned(),
            name: id.to_owned(),
            color: "auto".to_owned(),
            hex_color: String::new(),
            is_default,
            can_edit: true,
            owner_name: String::new(),
            owner_address: String::new(),
            shown: true,
        }
    }

    fn event(id: &str, start: &str, end: &str) -> EventRow {
        EventRow {
            id: id.to_owned(),
            subject: id.to_owned(),
            start: start.to_owned(),
            end: end.to_owned(),
            kind: "singleInstance".to_owned(),
            ..EventRow::default()
        }
    }

    fn db() -> AccountDb {
        let db = AccountDb::in_memory().unwrap();
        db.set_calendars(&[calendar("work", false), calendar("main", true)])
            .unwrap();
        db
    }

    fn ids(events: &[EventRow]) -> Vec<&str> {
        events.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn calendars_are_kept_as_the_server_has_them_and_hiding_one_is_this_computers_own() {
        let db = db();
        assert_eq!(db.calendars().unwrap()[0].id, "main");
        assert!(db.set_calendar_shown("work", false).unwrap());

        // The server's list comes again, renamed. Hidden stays hidden.
        let mut renamed = calendar("work", false);
        renamed.name = "Work".to_owned();
        db.set_calendars(&[calendar("main", true), renamed])
            .unwrap();
        let work = &db.calendars().unwrap()[1];
        assert_eq!((work.name.as_str(), work.shown), ("Work", false));
    }

    #[test]
    fn a_calendar_the_server_no_longer_has_goes_with_its_events() {
        let db = db();
        db.apply_events(
            "work",
            &[event("a", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z")],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();
        db.apply_events(
            "main",
            &[event("b", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z")],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();

        db.set_calendars(&[calendar("main", true)]).unwrap();

        assert_eq!(db.calendars().unwrap().len(), 1);
        assert!(db.event("a").unwrap().is_none());
        assert!(db.event("b").unwrap().is_some());
    }

    #[test]
    fn events_are_found_by_the_time_they_overlap_earliest_first() {
        let db = db();
        db.apply_events(
            "main",
            &[
                event("lunch", "2026-10-20T02:00:00Z", "2026-10-20T03:00:00Z"),
                event("early", "2026-10-19T22:00:00Z", "2026-10-19T23:00:00Z"),
                event("overnight", "2026-10-19T13:00:00Z", "2026-10-19T15:00:00Z"),
                event(
                    "ends-as-it-begins",
                    "2026-10-19T13:00:00Z",
                    "2026-10-19T14:00:00Z",
                ),
                event("next-day", "2026-10-20T14:00:00Z", "2026-10-20T15:00:00Z"),
            ],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();

        // Tuesday 20 October in Brisbane: from 14:00 UTC on the 19th.
        let day = db
            .events_between("2026-10-19T14:00:00Z", "2026-10-20T14:00:00Z", true)
            .unwrap();

        // One that runs past midnight is in; one that ends as the day
        // begins, or begins as it ends, is not.
        assert_eq!(ids(&day), ["overnight", "early", "lunch"]);
    }

    #[test]
    fn a_hidden_calendars_events_are_left_out_only_where_that_is_asked() {
        let db = db();
        db.apply_events(
            "work",
            &[event("a", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z")],
            &[],
            "next",
            Round::Finished,
        )
        .unwrap();
        db.set_calendar_shown("work", false).unwrap();
        let between = |only_shown| {
            db.events_between("2026-10-20T00:00:00Z", "2026-10-21T00:00:00Z", only_shown)
                .unwrap()
                .len()
        };

        assert_eq!((between(true), between(false)), (0, 1));
    }

    #[test]
    fn a_full_pass_takes_away_what_it_did_not_see_and_a_change_replaces_an_event_whole() {
        let db = db();
        let first = [
            event("a", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z"),
            event("b", "2026-10-21T01:00:00Z", "2026-10-21T02:00:00Z"),
        ];
        db.apply_events("main", &first, &[], "next", Round::Finished)
            .unwrap();

        // Moved an hour later, and its reminder taken off.
        let mut moved = event("a", "2026-10-20T02:00:00Z", "2026-10-20T03:00:00Z");
        moved.reminder_minutes = None;
        db.apply_events("main", &[moved], &["b".to_owned()], "next", Round::Finished)
            .unwrap();
        assert_eq!(
            db.event("a").unwrap().unwrap().start,
            "2026-10-20T02:00:00Z"
        );
        assert!(db.event("b").unwrap().is_none());

        // The server asks for a fresh start; what the new pass does not name goes.
        db.restart_events_sync("main").unwrap();
        db.apply_events(
            "main",
            &[event("c", "2026-10-22T01:00:00Z", "2026-10-22T02:00:00Z")],
            &[],
            "more",
            Round::Continues,
        )
        .unwrap();
        assert!(
            db.event("a").unwrap().is_some(),
            "kept until the pass is done"
        );
        db.apply_events("main", &[], &[], "next", Round::Finished)
            .unwrap();
        assert!(db.event("a").unwrap().is_none());
        assert!(db.event("c").unwrap().is_some());
    }

    #[test]
    fn a_series_master_is_kept_but_is_not_itself_something_that_happens() {
        let db = db();
        let mut master = event("series", "2026-10-20T01:00:00Z", "2026-10-20T02:00:00Z");
        master.kind = "seriesMaster".to_owned();
        db.apply_events("main", &[master], &[], "next", Round::Finished)
            .unwrap();

        assert!(db.event("series").unwrap().is_some());
        assert!(
            db.events_between("2026-10-20T00:00:00Z", "2026-10-21T00:00:00Z", true)
                .unwrap()
                .is_empty()
        );
    }
}
