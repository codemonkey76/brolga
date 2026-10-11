//! Keeping an account's calendars and events in step with the server.
//!
//! Events are read as a calendar view: the server places each occurrence of
//! a repeating event in time, and what is kept is what it says. The view is
//! of a window of time about now. The default calendar has a delta, so
//! after the first read only what changed is fetched; Graph has none for
//! any other calendar, so those are read whole, and less often.

use std::sync::Arc;

use brolga_graph::{DeltaItem, Event, GraphClient, GraphError, Next};
use brolga_store::{AccountDb, CalendarRow, EventRow, Round};
use jiff::{Timestamp, ToSpan};

use crate::SyncError;
use crate::mail::Synced;

/// How far back and how far ahead events are kept.
const DAYS_BACK: i64 = 365;
const DAYS_AHEAD: i64 = 730;
/// The window is of time about now, so now and then it is moved on. Until
/// then the delta carries on within the window it began with.
const MOVE_WINDOW_AFTER_DAYS: i64 = 30;
/// How often a calendar that has no delta is read again.
const READ_WHOLE_EVERY_MINUTES: i64 = 15;
/// How many fresh starts the server may ask for in one sync.
const MOST_RESTARTS: u32 = 3;

pub struct CalendarSync {
    graph: Arc<GraphClient>,
    db: Arc<AccountDb>,
    page_size: u32,
}

impl CalendarSync {
    pub fn new(graph: Arc<GraphClient>, db: Arc<AccountDb>) -> Self {
        Self {
            graph,
            db,
            page_size: 100,
        }
    }

    /// Ask for this many events a page. Smaller pages save progress more often.
    pub fn with_page_size(mut self, page_size: u32) -> Self {
        self.page_size = page_size.max(1);
        self
    }

    /// Bring the calendars and their events up to date, as of `now`.
    pub async fn sync_all(&self, now: Timestamp) -> Result<Synced, SyncError> {
        let mut synced = Synced::default();

        let calendars = self.sync_calendars().await?;
        synced.changed += u32::try_from(calendars.len()).unwrap_or(u32::MAX);

        let window = self.window(now, &calendars)?;
        for calendar in &calendars {
            if calendar.is_default {
                synced.add(self.sync_default(&calendar.id, &window).await?);
            } else if self.due(&calendar.id)? {
                synced.add(self.read_whole(&calendar.id, &window).await?);
            }
        }

        Ok(synced)
    }

    /// Read which calendars there are, and keep them. What was in one that
    /// is no longer there goes with it.
    pub async fn sync_calendars(&self) -> Result<Vec<CalendarRow>, SyncError> {
        let calendars: Vec<CalendarRow> = self
            .graph
            .calendars()
            .await?
            .into_iter()
            .map(|calendar| {
                let owner = calendar.owner.unwrap_or_default();
                CalendarRow {
                    id: calendar.id,
                    name: calendar.name.unwrap_or_default(),
                    color: calendar.color.unwrap_or_else(|| "auto".to_owned()),
                    hex_color: calendar.hex_color.unwrap_or_default(),
                    is_default: calendar.is_default_calendar.unwrap_or(false),
                    can_edit: calendar.can_edit.unwrap_or(false),
                    owner_name: owner.name.unwrap_or_default(),
                    owner_address: owner.address.unwrap_or_default(),
                    shown: true,
                }
            })
            .collect();
        self.db.set_calendars(&calendars)?;

        Ok(calendars)
    }

    /// Bring one calendar's events up to date, whether or not it is due:
    /// for straight after something in it was changed from here.
    pub async fn sync_calendar(
        &self,
        calendar_id: &str,
        now: Timestamp,
    ) -> Result<Synced, SyncError> {
        let calendars = self.db.calendars()?;
        let Some(calendar) = calendars.iter().find(|each| each.id == calendar_id) else {
            return Ok(Synced::default());
        };
        let window = self.window(now, &calendars)?;

        if calendar.is_default {
            self.sync_default(&calendar.id, &window).await
        } else {
            self.read_whole(&calendar.id, &window).await
        }
    }

    /// The window events are kept for, as two moments in UTC. It is set
    /// when first needed and kept, so that a delta carries on within the
    /// window it began with; after a month it is moved on to be about now
    /// again, and every calendar is read afresh.
    fn window(
        &self,
        now: Timestamp,
        calendars: &[CalendarRow],
    ) -> Result<(String, String), SyncError> {
        let day = |moment: Timestamp| moment.strftime("%Y-%m-%dT00:00:00").to_string();
        let around = |start: Timestamp| -> Result<(String, String), SyncError> {
            let end = start
                .checked_add((24 * (DAYS_BACK + DAYS_AHEAD)).hours())
                .map_err(|e| GraphError::Decode(e.to_string()))?;
            Ok((day(start), day(end)))
        };

        if let Some(kept) = self.db.setting("events_window_start")?
            && let Ok(start) = format!("{kept}Z").parse::<Timestamp>()
        {
            let age_days = now.duration_since(start).as_hours() / 24 - DAYS_BACK;
            if (0..MOVE_WINDOW_AFTER_DAYS).contains(&age_days) {
                return around(start);
            }
        }

        let start = now
            .checked_sub((24 * DAYS_BACK).hours())
            .map_err(|e| GraphError::Decode(e.to_string()))?;
        let window = around(start)?;
        self.db.set_setting("events_window_start", &window.0)?;
        for calendar in calendars {
            self.db.restart_events_sync(&calendar.id)?;
        }

        Ok(window)
    }

    /// Whether a calendar with no delta is due to be read again.
    fn due(&self, calendar_id: &str) -> Result<bool, SyncError> {
        let state = self.db.events_sync(calendar_id)?;
        // A read that was cut short is carried on with at once.
        if !state.complete || state.reconciling {
            return Ok(true);
        }
        let last = self
            .db
            .events_synced_at(calendar_id)?
            .and_then(|at| at.parse::<Timestamp>().ok());

        // By the clock on the wall, which is what the time it was last
        // read was written down by.
        Ok(last.is_none_or(|last| {
            Timestamp::now().duration_since(last).as_mins() >= READ_WHOLE_EVERY_MINUTES
        }))
    }

    /// The default calendar: a first read of the whole window, and after
    /// that only what has changed.
    async fn sync_default(
        &self,
        calendar_id: &str,
        window: &(String, String),
    ) -> Result<Synced, SyncError> {
        let mut synced = Synced::default();
        let mut restarts = 0;

        loop {
            let state = self.db.events_sync(calendar_id)?;
            let page = match self
                .graph
                .events_delta(
                    state.link.as_deref(),
                    (&window.0, &window.1),
                    self.page_size,
                )
                .await
            {
                Ok(page) => page,
                Err(GraphError::SyncStateGone) => {
                    restarts += 1;
                    if restarts > MOST_RESTARTS {
                        return Err(SyncError::NeverSettled("the calendar".to_owned()));
                    }
                    synced.restarts += 1;
                    self.db.restart_events_sync(calendar_id)?;
                    continue;
                }
                Err(other) => return Err(other.into()),
            };

            let finished = self.apply(calendar_id, page.items, &page.next, &mut synced)?;
            // A full read is followed at once by the round that carries
            // what changed while it ran.
            if finished && !state.reconciling {
                return Ok(synced);
            }
        }
    }

    /// Any other calendar: the whole window, read again from the start, and
    /// what that read does not name taken away when it is done.
    async fn read_whole(
        &self,
        calendar_id: &str,
        window: &(String, String),
    ) -> Result<Synced, SyncError> {
        let mut synced = Synced::default();
        let state = self.db.events_sync(calendar_id)?;
        // A read cut short has a place to carry on from. A finished one
        // has not, and begins again.
        if state.complete && !state.reconciling {
            self.db.restart_events_sync(calendar_id)?;
        }

        loop {
            let state = self.db.events_sync(calendar_id)?;
            let link = state.link.as_deref().filter(|link| !link.is_empty());
            let page = match self
                .graph
                .calendar_view(calendar_id, link, (&window.0, &window.1), self.page_size)
                .await
            {
                Ok(page) => page,
                // The calendar went since the list of them was read. Its
                // events go when the next sync reads the list.
                Err(GraphError::Refused { status: 404, .. }) => return Ok(synced),
                Err(other) => return Err(other.into()),
            };

            if self.apply(calendar_id, page.items, &page.next, &mut synced)? {
                return Ok(synced);
            }
        }
    }

    /// Write a page and where it leaves the sync. True if it ended the round.
    fn apply(
        &self,
        calendar_id: &str,
        items: Vec<DeltaItem<Event>>,
        next: &Next,
        synced: &mut Synced,
    ) -> Result<bool, SyncError> {
        let mut changed = Vec::new();
        let mut removed = Vec::new();
        for item in items {
            match item {
                // An event whose times cannot be read cannot be placed, and
                // is left out, not shown at a wrong time.
                DeltaItem::Changed(event) => changed.extend(event_row(event)),
                DeltaItem::Removed { id } => removed.push(id),
            }
        }
        let round = match next {
            Next::Page(_) => Round::Continues,
            Next::Done(_) => Round::Finished,
        };
        self.db
            .apply_events(calendar_id, &changed, &removed, next.url(), round)?;
        synced.pages += 1;
        synced.changed += u32::try_from(changed.len()).unwrap_or(u32::MAX);
        synced.removed += u32::try_from(removed.len()).unwrap_or(u32::MAX);

        Ok(round == Round::Finished)
    }
}

/// An event as it is kept. Nothing if its start or end cannot be read.
pub fn event_row(event: Event) -> Option<EventRow> {
    let moment = |at: &brolga_graph::DateTimeTimeZone| {
        brolga_calendar::utc(&at.date_time, at.time_zone.as_deref().unwrap_or("UTC"))
    };
    let start = moment(event.start.as_ref()?)?;
    let end = moment(event.end.as_ref()?)?;
    let organizer = event.organizer.map(|o| o.email_address).unwrap_or_default();
    let attendees: Vec<serde_json::Value> = event
        .attendees
        .unwrap_or_default()
        .into_iter()
        .map(|attendee| {
            let who = attendee.email_address.unwrap_or_default();
            serde_json::json!({
                "name": who.name,
                "address": who.address,
                "kind": attendee.kind.unwrap_or_else(|| "required".to_owned()),
                "response": attendee.status.and_then(|s| s.response).unwrap_or_else(|| "none".to_owned()),
            })
        })
        .collect();

    Some(EventRow {
        id: event.id,
        calendar_id: String::new(),
        subject: event.subject.unwrap_or_default(),
        preview: event.body_preview.unwrap_or_default(),
        location: event
            .location
            .and_then(|l| l.display_name)
            .unwrap_or_default(),
        start,
        end,
        is_all_day: event.is_all_day.unwrap_or(false),
        start_zone: event.original_start_time_zone.unwrap_or_default(),
        end_zone: event.original_end_time_zone.unwrap_or_default(),
        show_as: event.show_as.unwrap_or_else(|| "busy".to_owned()),
        sensitivity: event.sensitivity.unwrap_or_else(|| "normal".to_owned()),
        is_cancelled: event.is_cancelled.unwrap_or(false),
        is_organizer: event.is_organizer.unwrap_or(false),
        organizer_name: organizer.name.unwrap_or_default(),
        organizer_address: organizer.address.unwrap_or_default(),
        response: event
            .response_status
            .and_then(|s| s.response)
            .unwrap_or_else(|| "none".to_owned()),
        kind: event.kind.unwrap_or_else(|| "singleInstance".to_owned()),
        series_master_id: event.series_master_id,
        categories: serde_json::Value::from(event.categories.unwrap_or_default()).to_string(),
        // A reminder that is off is no reminder, whatever minutes it carries.
        reminder_minutes: event
            .reminder_minutes_before_start
            .filter(|_| event.is_reminder_on.unwrap_or(false)),
        join_url: event
            .online_meeting
            .and_then(|m| m.join_url)
            .unwrap_or_default(),
        web_link: event.web_link.unwrap_or_default(),
        change_key: event.change_key.unwrap_or_default(),
        attendees: serde_json::Value::Array(attendees).to_string(),
    })
}
