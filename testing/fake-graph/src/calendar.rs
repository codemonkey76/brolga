//! A made-up calendar, and Graph's ways of asking about it.
//!
//! As on the real service, a calendar view gives each occurrence of a
//! repeating event as an event of its own, placed in time. The placing is
//! done with `brolga-calendar`, the same code the client uses offline; the
//! golden tests in that crate are what hold it to the right answers.

use std::collections::BTreeMap;

use brolga_calendar::{Series, occurrences};
use jiff::Timestamp;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct FakeCalendar {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub hex_color: String,
}

/// An event as it is made: a single one, or a series with how it repeats.
#[derive(Clone, Debug)]
pub struct FakeEvent {
    pub subject: String,
    /// UTC: `2026-10-20T03:00:00Z`. For a series, taken from the series.
    pub start: String,
    pub end: String,
    pub is_all_day: bool,
    /// The zone it was made in, as Windows names it.
    pub zone: String,
    pub location: String,
    pub show_as: String,
    pub reminder_minutes: Option<i64>,
    pub is_cancelled: bool,
    pub series: Option<Series>,
    /// Occurrences of a series that were cancelled, by the day they fell
    /// on in the series' own zone, as `2026-10-20`.
    pub cancelled_days: Vec<String>,
    /// Occurrences that were moved: the day one fell on, and its new start and end in UTC.
    pub moved: Vec<(String, String, String)>,
    /// How the series ran before it was changed, so that occurrences it
    /// no longer has can be named as gone.
    pub former: Vec<Series>,
    /// Occurrences changed on their own: the day one fell on, and what of
    /// it differs from the series, as Graph names it.
    pub overrides: Vec<(String, Value)>,
    /// Who is invited: `(name, address, kind, response)`.
    pub attendees: Vec<(String, String, String, String)>,
    /// It can be joined online.
    pub online: bool,
    pub sensitivity: String,
    pub categories: Vec<String>,
    /// What it says, as HTML or plain words.
    pub body: Option<String>,
    /// Who called the meeting, `(name, address)`, where it was not the
    /// owner of the mailbox.
    pub organizer: Option<(String, String)>,
    /// The owner's answer to it: `notResponded`, `accepted`,
    /// `tentativelyAccepted` or `declined`.
    pub response: String,
}

impl FakeEvent {
    /// A single event between two moments in UTC.
    pub fn new(subject: &str, start: &str, end: &str) -> Self {
        Self {
            subject: subject.to_owned(),
            start: start.to_owned(),
            end: end.to_owned(),
            is_all_day: false,
            zone: "E. Australia Standard Time".to_owned(),
            location: String::new(),
            show_as: "busy".to_owned(),
            reminder_minutes: Some(15),
            is_cancelled: false,
            series: None,
            cancelled_days: Vec::new(),
            moved: Vec::new(),
            former: Vec::new(),
            overrides: Vec::new(),
            attendees: Vec::new(),
            online: false,
            sensitivity: "normal".to_owned(),
            categories: Vec::new(),
            body: None,
            organizer: None,
            response: "notResponded".to_owned(),
        }
    }

    /// An event that repeats.
    pub fn repeating(subject: &str, series: Series, windows_zone: &str) -> Self {
        let mut event = Self::new(subject, "", "");
        event.zone = windows_zone.to_owned();
        event.series = Some(series);
        event
    }

    pub fn all_day(mut self) -> Self {
        self.is_all_day = true;
        self
    }

    /// Make it a meeting someone else called, not yet answered.
    pub fn invited_by(mut self, name: &str, address: &str) -> Self {
        self.organizer = Some((name.to_owned(), address.to_owned()));
        self.show_as = "tentative".to_owned();
        self
    }

    pub fn at(mut self, location: &str) -> Self {
        self.location = location.to_owned();
        self
    }

    pub fn cancel_on(mut self, day: &str) -> Self {
        self.cancelled_days.push(day.to_owned());
        self
    }

    pub fn move_on(mut self, day: &str, start: &str, end: &str) -> Self {
        self.moved
            .push((day.to_owned(), start.to_owned(), end.to_owned()));
        self
    }
}

struct Stored {
    calendar: String,
    event: FakeEvent,
    deleted: bool,
}

#[derive(Default)]
pub struct Calendars {
    calendars: Vec<FakeCalendar>,
    events: BTreeMap<String, Stored>,
    /// When other people are busy, for "calendar: getSchedule": each
    /// address, and its (start, end, status) in UTC.
    pub busy: BTreeMap<String, Vec<(String, String, String)>>,
    /// Every message a meeting's organiser has had sent, oldest first:
    /// (event, to whom, what: `invitation`, `update` or `cancellation`, and
    /// the organiser's words with a cancellation).
    pub sent: Vec<(String, String, String, String)>,
    /// The events made with each `transactionId`, so that a request that
    /// arrives twice makes one event.
    made: BTreeMap<String, String>,
    /// Every change to an event, oldest first: (when, which).
    log: Vec<(u64, String)>,
    seq: u64,
    /// Bumped to make every delta token given out so far no good.
    epoch: u64,
    next_id: u32,
}

/// What a view of events answers.
pub enum View {
    Page(Value),
    /// The token is no longer accepted: sync again from the start.
    Gone,
    NoSuchCalendar,
}

/// Write what a request gives into an event, as "Create event" and
/// "Update event" do: what is not given is left as it was.
fn write(event: &mut FakeEvent, body: &Value) -> Result<(), &'static str> {
    const INVALID: &str = "ErrorPropertyValidationFailure";
    let words = |value: &Value| value.as_str().map(str::to_owned);

    if let Some(subject) = words(&body["subject"]) {
        event.subject = subject;
    }
    if let Some(location) = words(&body["location"]["displayName"]) {
        event.location = location;
    }
    if let Some(show_as) = words(&body["showAs"]) {
        if !["free", "tentative", "busy", "oof", "workingElsewhere"].contains(&show_as.as_str()) {
            return Err(INVALID);
        }
        event.show_as = show_as;
    }
    if let Some(sensitivity) = words(&body["sensitivity"]) {
        if !["normal", "personal", "private", "confidential"].contains(&sensitivity.as_str()) {
            return Err(INVALID);
        }
        event.sensitivity = sensitivity;
    }
    if let Some(categories) = body["categories"].as_array() {
        event.categories = categories.iter().filter_map(words).collect();
    }
    if let Some(content) = words(&body["body"]["content"]) {
        event.body = Some(content);
    }
    if let Some(invited) = body["attendees"].as_array() {
        let mut all = Vec::new();
        for each in invited {
            let address = words(&each["emailAddress"]["address"])
                .filter(|address| address.contains('@'))
                .ok_or(INVALID)?;
            let kind = words(&each["type"]).unwrap_or_else(|| "required".to_owned());
            if !["required", "optional", "resource"].contains(&kind.as_str()) {
                return Err(INVALID);
            }
            // Someone already invited keeps the answer they gave.
            let answered = event
                .attendees
                .iter()
                .find(|had| had.1.eq_ignore_ascii_case(&address))
                .map_or_else(|| "none".to_owned(), |had| had.3.clone());
            all.push((
                words(&each["emailAddress"]["name"]).unwrap_or_default(),
                address,
                kind,
                answered,
            ));
        }
        event.attendees = all;
    }
    // "event resource type": once a meeting is online it stays so.
    if body["isOnlineMeeting"].as_bool() == Some(true) {
        event.online = true;
    }
    if let Some(all_day) = body["isAllDay"].as_bool() {
        event.is_all_day = all_day;
    }
    if let Some(minutes) = body["reminderMinutesBeforeStart"].as_i64() {
        event.reminder_minutes = Some(minutes);
    }
    if body["isReminderOn"].as_bool() == Some(false) {
        event.reminder_minutes = None;
    }
    for (name, kept) in [("start", &mut event.start), ("end", &mut event.end)] {
        let given = &body[name];
        if given.is_null() {
            continue;
        }
        let (Some(local), Some(zone)) = (given["dateTime"].as_str(), given["timeZone"].as_str())
        else {
            return Err(INVALID);
        };
        if brolga_calendar::iana_zone(zone).is_none() {
            return Err(INVALID);
        }
        if name == "start" {
            event.zone = zone.to_owned();
        }
        // An all-day event is days, not moments: its start and end must
        // both be at midnight, and are kept as the days that were written.
        *kept = if event.is_all_day {
            if !local.contains("T00:00:00") {
                return Err(INVALID);
            }
            format!("{}Z", local.split('.').next().unwrap_or(local))
        } else {
            brolga_calendar::utc(local, zone).ok_or(INVALID)?
        };
    }
    if let (Some(start), Some(end)) = (stamp(&event.start), stamp(&event.end))
        && end < start
    {
        return Err(INVALID);
    }

    // How it repeats. The series' first start and end are by the clock in
    // the event's zone: as just given, or as the series already had them.
    let zone = brolga_calendar::iana_zone(&event.zone)
        .ok_or(INVALID)?
        .to_owned();
    let by_clock = |name: &str, kept: &str, all_day: bool| -> Option<jiff::civil::DateTime> {
        if let Some(local) = body[name]["dateTime"].as_str() {
            return local.split('.').next()?.parse().ok();
        }
        let moment = stamp(kept)?;
        if all_day {
            return Some(moment.to_zoned(jiff::tz::TimeZone::UTC).datetime());
        }
        Some(
            moment
                .to_zoned(jiff::tz::TimeZone::get(&zone).ok()?)
                .datetime(),
        )
    };
    let given = &body["recurrence"];
    if given.is_object() {
        let was = event.series.take();
        let (start, end) = match (&was, body["start"].is_null()) {
            (Some(was), true) => (was.start, was.end),
            _ => (
                by_clock("start", &event.start, event.is_all_day).ok_or(INVALID)?,
                by_clock("end", &event.end, event.is_all_day).ok_or(INVALID)?,
            ),
        };
        let series = Series::from_graph(start, end, &zone, given).ok_or(INVALID)?;
        // "recurrenceRange": the range starts on the day the event starts.
        if series.from != start.date()
            || occurrences(&series, Timestamp::MIN, Timestamp::MAX, 1).is_err()
        {
            return Err(INVALID);
        }
        event.former.extend(was);
        event.series = Some(series);
    } else if let (Some(series), false) = (&mut event.series, body["start"].is_null()) {
        // The whole series moved: its first occurrence is when it is now said to start.
        let (start, end) = (
            by_clock("start", &event.start, event.is_all_day).ok_or(INVALID)?,
            by_clock("end", &event.end, event.is_all_day).ok_or(INVALID)?,
        );
        let was = series.clone();
        series.start = start;
        series.end = end;
        series.from = start.date();
        event.former.push(was);
    }

    Ok(())
}

fn stamp(text: &str) -> Option<Timestamp> {
    // The window comes as `2026-01-01T00:00:00`, with or without a `Z`.
    let text = text.trim_end_matches('Z');
    format!("{text}Z").parse().ok()
}

/// A moment some days earlier or later.
fn wider(moment: Timestamp, days: i64) -> Timestamp {
    moment
        .checked_add(jiff::Span::new().hours(24 * days))
        .unwrap_or(moment)
}

fn wider_minutes(moment: Timestamp, minutes: i64) -> Timestamp {
    moment
        .checked_add(jiff::Span::new().minutes(minutes))
        .unwrap_or(moment)
}

fn utc(moment: Timestamp) -> String {
    moment.strftime("%Y-%m-%dT%H:%M:%S.0000000").to_string()
}

impl Calendars {
    pub fn with_default() -> Self {
        let mut calendars = Self::default();
        calendars.calendars.push(FakeCalendar {
            id: "calendar-default".to_owned(),
            name: "Calendar".to_owned(),
            is_default: true,
            hex_color: String::new(),
        });
        calendars
    }

    pub fn add_calendar(&mut self, name: &str, hex_color: &str) -> String {
        let id = format!("calendar-{}", self.calendars.len() + 1);
        self.calendars.push(FakeCalendar {
            id: id.clone(),
            name: name.to_owned(),
            is_default: false,
            hex_color: hex_color.to_owned(),
        });
        id
    }

    pub fn remove_calendar(&mut self, id: &str) {
        self.calendars.retain(|c| c.id != id);
        self.events.retain(|_, stored| stored.calendar != id);
    }

    pub fn default_calendar(&self) -> String {
        self.calendars
            .iter()
            .find(|c| c.is_default)
            .map(|c| c.id.clone())
            .unwrap_or_default()
    }

    fn touched(&mut self, id: &str) {
        self.seq += 1;
        self.log.push((self.seq, id.to_owned()));
    }

    pub fn add_event(&mut self, calendar: &str, event: FakeEvent) -> String {
        self.next_id += 1;
        let id = format!("event-{:04}", self.next_id);
        self.events.insert(
            id.clone(),
            Stored {
                calendar: calendar.to_owned(),
                event,
                deleted: false,
            },
        );
        self.touched(&id);
        id
    }

    pub fn change_event(&mut self, id: &str, change: impl FnOnce(&mut FakeEvent)) {
        if let Some(stored) = self.events.get_mut(id) {
            change(&mut stored.event);
            self.touched(id);
        }
    }

    pub fn delete_event(&mut self, id: &str) {
        if let Some(stored) = self.events.get_mut(id) {
            stored.deleted = true;
            self.touched(id);
        }
    }

    /// "calendar: getSchedule" (SOURCES.md): for each address, what it has
    /// on between two moments, and a digit for each slot of the time: 0
    /// free, 1 tentative, 2 busy, 3 out of office. The owner's own is read
    /// from the default calendar; an address nobody has gives an error.
    pub fn schedules(&self, body: &Value) -> Option<Value> {
        let moment = |at: &Value| {
            stamp(&brolga_calendar::utc(
                at["dateTime"].as_str()?,
                at["timeZone"].as_str()?,
            )?)
        };
        let (after, before) = (moment(&body["startTime"])?, moment(&body["endTime"])?);
        let interval = body["availabilityViewInterval"]
            .as_i64()
            .unwrap_or(30)
            .clamp(5, 1440);
        let slots = (before.duration_since(after).as_mins() / interval).max(0);
        let default = self.default_calendar();

        let all: Vec<Value> = body["schedules"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(|address| {
                let on: Vec<(Timestamp, Timestamp, String)> = if address == "robin@example.test" {
                    self.events
                        .iter()
                        .filter(|(_, stored)| stored.calendar == default && !stored.deleted)
                        .flat_map(|(id, _)| self.instances(id, after, before))
                        .filter_map(|v| {
                            let at = |name: &str| stamp(&format!("{}Z", v[name]["dateTime"].as_str()?.split('.').next()?));
                            Some((at("start")?, at("end")?, v["showAs"].as_str()?.to_owned()))
                        })
                        .collect()
                } else if let Some(busy) = self.busy.get(address) {
                    busy.iter()
                        .filter_map(|(start, end, status)| Some((stamp(start)?, stamp(end)?, status.clone())))
                        .filter(|(start, end, _)| *start < before && *end > after)
                        .collect()
                } else {
                    return json!({
                        "scheduleId": address,
                        "availabilityView": "",
                        "scheduleItems": [],
                        "error": { "message": "The recipient was not found.", "responseCode": "ErrorMailRecipientNotFound" }
                    });
                };
                let digit = |status: &str| match status {
                    "tentative" => '1',
                    "busy" => '2',
                    "oof" => '3',
                    _ => '0',
                };
                let view: String = (0..slots)
                    .map(|slot| {
                        let from = wider_minutes(after, slot * interval);
                        let to = wider_minutes(after, (slot + 1) * interval);
                        on.iter()
                            .filter(|(start, end, _)| *start < to && *end > from)
                            .map(|(_, _, status)| digit(status))
                            .max()
                            .unwrap_or('0')
                    })
                    .collect();
                json!({
                    "scheduleId": address,
                    "availabilityView": view,
                    "scheduleItems": on.iter().map(|(start, end, status)| json!({
                        "status": status,
                        "start": { "dateTime": utc(*start), "timeZone": "UTC" },
                        "end": { "dateTime": utc(*end), "timeZone": "UTC" }
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();

        Some(json!({ "value": all }))
    }

    /// "Create event" (SOURCES.md): the id of the event made, or of the
    /// one already made by a request with the same `transactionId`.
    pub fn create(&mut self, calendar: &str, body: &Value) -> Result<String, &'static str> {
        if !self.calendars.iter().any(|each| each.id == calendar) {
            return Err("ErrorItemNotFound");
        }
        let transaction = body["transactionId"].as_str().map(str::to_owned);
        if let Some(made) = transaction.as_ref().and_then(|id| self.made.get(id)) {
            return Ok(made.clone());
        }
        let mut event = FakeEvent::new("", "", "");
        event.reminder_minutes = Some(15);
        write(&mut event, body)?;
        if stamp(&event.start).is_none() || stamp(&event.end).is_none() {
            return Err("ErrorPropertyValidationFailure");
        }
        let invited = event.attendees.clone();
        let id = self.add_event(calendar, event);
        if let Some(transaction) = transaction {
            self.made.insert(transaction, id.clone());
        }
        // "Create event": everyone invited is sent an invitation.
        for (_, address, _, _) in invited {
            self.sent
                .push((id.clone(), address, "invitation".to_owned(), String::new()));
        }

        Ok(id)
    }

    /// "Update event" (SOURCES.md): only what is given is changed.
    pub fn update(&mut self, id: &str, body: &Value) -> Result<(), &'static str> {
        if let Some((own, day)) = self.occurrence(id) {
            let stored = self.events.get_mut(&own).ok_or("ErrorItemNotFound")?;
            // An occurrence changed on its own becomes an exception: moved,
            // or differing in what it says.
            if !body["start"].is_null() || !body["end"].is_null() {
                let moment = |at: &Value| {
                    brolga_calendar::utc(at["dateTime"].as_str()?, at["timeZone"].as_str()?)
                };
                let (Some(start), Some(end)) = (moment(&body["start"]), moment(&body["end"]))
                else {
                    return Err("ErrorPropertyValidationFailure");
                };
                stored.event.moved.retain(|(on, _, _)| *on != day);
                stored.event.moved.push((day.clone(), start, end));
            }
            let mut differs = serde_json::Map::new();
            for name in ["subject", "location", "showAs", "sensitivity", "categories"] {
                if !body[name].is_null() {
                    differs.insert(name.to_owned(), body[name].clone());
                }
            }
            if !differs.is_empty() {
                let mut all = match stored.event.overrides.iter().position(|(on, _)| *on == day) {
                    Some(at) => stored.event.overrides.remove(at).1,
                    None => json!({}),
                };
                for (name, value) in differs {
                    all[name.as_str()] = value;
                }
                stored.event.overrides.push((day, all));
            }
            self.touched(&own);
            return Ok(());
        }
        let Some(stored) = self.events.get_mut(id).filter(|stored| !stored.deleted) else {
            return Err("ErrorItemNotFound");
        };
        let mut changed = stored.event.clone();
        write(&mut changed, body)?;
        // "Update event": a change to who is invited alone is told only to
        // those it changes; any other change is told to everyone.
        let addresses = |event: &FakeEvent| -> Vec<String> {
            event.attendees.iter().map(|each| each.1.clone()).collect()
        };
        let (before, after) = (addresses(&stored.event), addresses(&changed));
        let only_people = body
            .as_object()
            .is_some_and(|given| given.keys().all(|name| name == "attendees"));
        let mine = stored.event.organizer.is_none();
        stored.event = changed;
        if mine {
            for address in &after {
                let what = if !before.contains(address) {
                    "invitation"
                } else if only_people {
                    continue;
                } else {
                    "update"
                };
                self.sent.push((
                    id.to_owned(),
                    address.clone(),
                    what.to_owned(),
                    String::new(),
                ));
            }
            for address in before.iter().filter(|address| !after.contains(address)) {
                self.sent.push((
                    id.to_owned(),
                    address.clone(),
                    "cancellation".to_owned(),
                    String::new(),
                ));
            }
        }
        self.touched(id);

        Ok(())
    }

    /// "event: cancel" (SOURCES.md): for the organiser alone. Everyone
    /// invited is told, with the organiser's words, and the event goes.
    pub fn cancel(&mut self, id: &str, comment: &str) -> Result<(), &'static str> {
        let own = id.split('.').next().unwrap_or(id).to_owned();
        let Some(stored) = self.events.get(&own).filter(|stored| !stored.deleted) else {
            return Err("ErrorItemNotFound");
        };
        if stored.event.organizer.is_some() {
            return Err("organizer");
        }
        let invited: Vec<String> = stored
            .event
            .attendees
            .iter()
            .map(|each| each.1.clone())
            .collect();
        self.delete_quietly(id)?;
        for address in invited {
            self.sent.push((
                id.to_owned(),
                address,
                "cancellation".to_owned(),
                comment.to_owned(),
            ));
        }

        Ok(())
    }

    /// "Delete event" (SOURCES.md).
    /// The series and the day of an occurrence's id, if it is one and
    /// there is such an occurrence.
    fn occurrence(&self, id: &str) -> Option<(String, String)> {
        let (own, compact) = id.split_once('.')?;
        let stored = self.events.get(own).filter(|stored| !stored.deleted)?;
        stored.event.series.as_ref()?;
        let day = format!(
            "{}-{}-{}",
            compact.get(..4)?,
            compact.get(4..6)?,
            compact.get(6..8)?
        );

        (!stored.event.cancelled_days.contains(&day)).then(|| (own.to_owned(), day))
    }

    /// "Delete event" (SOURCES.md). A meeting the owner organised is
    /// cancelled for everyone invited.
    pub fn delete(&mut self, id: &str) -> Result<(), &'static str> {
        let own = id.split('.').next().unwrap_or(id).to_owned();
        let invited: Vec<String> = self
            .events
            .get(&own)
            .filter(|stored| stored.event.organizer.is_none())
            .map(|stored| {
                stored
                    .event
                    .attendees
                    .iter()
                    .map(|each| each.1.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.delete_quietly(id)?;
        for address in invited {
            self.sent.push((
                id.to_owned(),
                address,
                "cancellation".to_owned(),
                String::new(),
            ));
        }

        Ok(())
    }

    fn delete_quietly(&mut self, id: &str) -> Result<(), &'static str> {
        if let Some((own, day)) = self.occurrence(id) {
            if let Some(stored) = self.events.get_mut(&own) {
                stored.event.cancelled_days.push(day);
            }
            self.touched(&own);
            return Ok(());
        }
        if !self.events.get(id).is_some_and(|stored| !stored.deleted) {
            return Err("ErrorItemNotFound");
        }
        self.delete_event(id);

        Ok(())
    }

    /// Answer an invitation ("event: accept" and its fellows): `Err` says
    /// why not. A declined event leaves the calendar.
    pub fn respond(&mut self, id: &str, answer: &str) -> Result<(), &'static str> {
        let own = id.split('.').next().unwrap_or(id).to_owned();
        let Some(stored) = self.events.get_mut(&own).filter(|stored| !stored.deleted) else {
            return Err("ErrorItemNotFound");
        };
        if stored.event.organizer.is_none() {
            return Err("organizer");
        }
        match answer {
            "accept" => {
                stored.event.response = "accepted".to_owned();
                stored.event.show_as = "busy".to_owned();
            }
            "tentativelyAccept" => stored.event.response = "tentativelyAccepted".to_owned(),
            _ => {
                stored.event.response = "declined".to_owned();
                stored.deleted = true;
            }
        }
        self.touched(&own);

        Ok(())
    }

    pub fn expire_tokens(&mut self) {
        self.epoch += 1;
    }

    pub fn list(&self) -> Value {
        let list: Vec<Value> = self
            .calendars
            .iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "name": c.name,
                    "color": "auto",
                    "hexColor": c.hex_color,
                    "isDefaultCalendar": c.is_default,
                    "canEdit": true,
                    "owner": { "name": "Robin Example", "address": "robin@example.test" }
                })
            })
            .collect();

        json!({ "value": list })
    }

    /// What an event is in a view between two moments: itself, if it is a
    /// single event that overlaps them, or each of its occurrences that does.
    fn instances(&self, id: &str, after: Timestamp, before: Timestamp) -> Vec<Value> {
        let Some(stored) = self.events.get(id) else {
            return Vec::new();
        };
        let event = &stored.event;
        let shown = |instance_id: String, kind: &str, start: Timestamp, end: Timestamp| {
            json!({
                "id": instance_id,
                "subject": event.subject,
                "bodyPreview": "",
                "start": { "dateTime": utc(start), "timeZone": "UTC" },
                "end": { "dateTime": utc(end), "timeZone": "UTC" },
                "isAllDay": event.is_all_day,
                "originalStartTimeZone": event.zone,
                "originalEndTimeZone": event.zone,
                "location": { "displayName": event.location },
                "showAs": event.show_as,
                "sensitivity": event.sensitivity,
                "isCancelled": event.is_cancelled,
                "isOrganizer": event.organizer.is_none(),
                "organizer": { "emailAddress": match &event.organizer {
                    Some((name, address)) => json!({ "name": name, "address": address }),
                    None => json!({ "name": "Robin Example", "address": "robin@example.test" }),
                } },
                "attendees": event.attendees.iter().map(|(name, address, kind, response)| json!({
                    "type": kind,
                    "status": { "response": response, "time": "0001-01-01T00:00:00Z" },
                    "emailAddress": { "name": name, "address": address }
                })).collect::<Vec<_>>(),
                "responseStatus": { "response": if event.organizer.is_some() { event.response.as_str() } else { "organizer" } },
                "type": kind,
                "seriesMasterId": if kind == "singleInstance" { Value::Null } else { json!(id) },
                "categories": event.categories,
                "isReminderOn": event.reminder_minutes.is_some(),
                "reminderMinutesBeforeStart": event.reminder_minutes.unwrap_or(15),
                "isOnlineMeeting": event.online,
                "onlineMeetingProvider": if event.online { "teamsForBusiness" } else { "unknown" },
                "onlineMeeting": if event.online {
                    json!({ "joinUrl": format!("https://teams.example.test/meet/{id}") })
                } else {
                    Value::Null
                },
                "webLink": format!("https://outlook.example.test/calendar/item/{instance_id}"),
                "changeKey": format!("key-{}", self.seq),
            })
        };

        match &event.series {
            None => {
                let (Some(start), Some(end)) = (stamp(&event.start), stamp(&event.end)) else {
                    return Vec::new();
                };
                if start < before && end > after {
                    vec![shown(id.to_owned(), "singleInstance", start, end)]
                } else {
                    Vec::new()
                }
            }
            // Only the stretch asked about is worked through. A day either
            // side is taken in, for an occurrence moved into it from outside.
            Some(series) => occurrences(series, wider(after, -1), wider(before, 1), 20_000)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|occurrence| {
                    let day = occurrence.local_start.date().to_string();
                    if event.cancelled_days.contains(&day) {
                        return None;
                    }
                    let instance_id = format!("{id}.{}", day.replace('-', ""));
                    let moved = event.moved.iter().find(|(on, _, _)| *on == day);
                    let differs = event.overrides.iter().find(|(on, _)| *on == day);
                    let (kind, start, end) = match moved {
                        Some((_, start, end)) => ("exception", stamp(start)?, stamp(end)?),
                        None if differs.is_some() => {
                            ("exception", occurrence.start, occurrence.end)
                        }
                        None => ("occurrence", occurrence.start, occurrence.end),
                    };
                    (start < before && end > after).then(|| {
                        let mut instance = shown(instance_id, kind, start, end);
                        if let Some((_, Value::Object(changes))) = differs {
                            for (name, value) in changes {
                                instance[name.as_str()] = value.clone();
                            }
                        }
                        instance
                    })
                })
                .collect(),
        }
    }

    /// Every id an event could have had in a view: its own, or one for each
    /// day of the series. A deleted event is named as removed under each.
    fn all_ids(&self, id: &str, after: Timestamp, before: Timestamp) -> Vec<String> {
        let Some(stored) = self.events.get(id) else {
            return vec![id.to_owned()];
        };
        // A single event that was once a series, or a series that was once
        // otherwise, had other ids then.
        let mut all: Vec<String> = stored
            .event
            .series
            .iter()
            .chain(&stored.event.former)
            .flat_map(|series| {
                occurrences(series, wider(after, -1), wider(before, 1), 20_000).unwrap_or_default()
            })
            .map(|o| format!("{id}.{}", o.local_start.date().to_string().replace('-', "")))
            .collect();
        if stored.event.series.is_none() || all.is_empty() {
            all.push(id.to_owned());
        }
        all.sort();
        all.dedup();
        all
    }

    /// "event: delta" (SOURCES.md), for the default calendar. The window is
    /// given on the first request and carried in the tokens after it.
    pub fn delta(
        &self,
        query: &std::collections::HashMap<String, String>,
        page_size: usize,
        link: &str,
    ) -> View {
        let token = query.get("$skiptoken").or_else(|| query.get("$deltatoken"));
        let calendar = self.default_calendar();
        let page_size = page_size.max(1);

        // A token is `kind.epoch.since.offset.start.end`.
        let (kind, since, offset, window) = match token {
            None => {
                let (Some(start), Some(end)) =
                    (query.get("startDateTime"), query.get("endDateTime"))
                else {
                    return View::Page(
                        json!({ "error": "startDateTime and endDateTime are required" }),
                    );
                };
                ("s", self.seq, 0usize, (start.clone(), end.clone()))
            }
            Some(token) => {
                let parts: Vec<&str> = token.splitn(6, '~').collect();
                let number = |at: usize| parts.get(at).and_then(|p| p.parse::<u64>().ok());
                if number(1) != Some(self.epoch) {
                    return View::Gone;
                }
                let (Some(since), Some(offset), Some(start), Some(end)) =
                    (number(2), number(3), parts.get(4), parts.get(5))
                else {
                    return View::Gone;
                };
                (
                    if parts[0] == "s" { "s" } else { "d" },
                    since,
                    offset as usize,
                    ((*start).to_owned(), (*end).to_owned()),
                )
            }
        };
        let (Some(after), Some(before)) = (stamp(&window.0), stamp(&window.1)) else {
            return View::Gone;
        };
        let tail = format!("{}~{}", window.0, window.1);

        let entries: Vec<Value> = if kind == "s" {
            self.events
                .iter()
                .filter(|(_, stored)| stored.calendar == calendar && !stored.deleted)
                .flat_map(|(id, _)| self.instances(id, after, before))
                .collect()
        } else {
            // Each event changed since, once: gone, or as it now stands. An
            // occurrence that is no longer in the view is named as removed.
            let mut changed: Vec<&String> = self
                .log
                .iter()
                .filter(|(seq, _)| *seq > since)
                .map(|(_, id)| id)
                .collect();
            changed.sort();
            changed.dedup();
            changed
                .into_iter()
                .filter(|id| {
                    self.events
                        .get(*id)
                        .is_some_and(|stored| stored.calendar == calendar)
                })
                .flat_map(|id| {
                    let now: Vec<Value> = if self.events[id].deleted {
                        Vec::new()
                    } else {
                        self.instances(id, after, before)
                    };
                    let still: Vec<String> = now
                        .iter()
                        .filter_map(|v| v["id"].as_str().map(str::to_owned))
                        .collect();
                    let gone = self
                        .all_ids(id, after, before)
                        .into_iter()
                        .filter(move |each| !still.contains(each))
                        .map(|each| json!({ "id": each, "@removed": { "reason": "deleted" } }));
                    now.into_iter().chain(gone).collect::<Vec<_>>()
                })
                .collect()
        };

        let page: Vec<Value> = entries
            .iter()
            .skip(offset)
            .take(page_size)
            .cloned()
            .collect();
        let more = entries.len() > offset + page_size;
        let next = if more {
            json!(format!(
                "{link}?$skiptoken={kind}~{}~{since}~{}~{tail}",
                self.epoch,
                offset + page_size
            ))
        } else {
            Value::Null
        };
        // A round that ends hands back a token for what changes after the
        // moment the round began.
        let done = if more {
            Value::Null
        } else {
            let from = if kind == "s" { since } else { self.seq };
            json!(format!(
                "{link}?$deltatoken=d~{}~{from}~0~{tail}",
                self.epoch
            ))
        };

        let mut body = json!({ "value": page });
        if more {
            body["@odata.nextLink"] = next;
        } else {
            body["@odata.deltaLink"] = done;
        }
        View::Page(body)
    }

    /// "List calendarView" (SOURCES.md), for any calendar: no delta, a
    /// nextLink while there is more and nothing at the end.
    pub fn view(
        &self,
        calendar: &str,
        query: &std::collections::HashMap<String, String>,
        page_size: usize,
        link: &str,
    ) -> View {
        if !self.calendars.iter().any(|c| c.id == calendar) {
            return View::NoSuchCalendar;
        }
        let (Some(start), Some(end)) = (query.get("startDateTime"), query.get("endDateTime"))
        else {
            return View::Page(json!({ "error": "startDateTime and endDateTime are required" }));
        };
        let (Some(after), Some(before)) = (stamp(start), stamp(end)) else {
            return View::Page(json!({ "value": [] }));
        };
        let offset: usize = query.get("$skip").and_then(|s| s.parse().ok()).unwrap_or(0);
        let page_size = page_size.max(1);

        let entries: Vec<Value> = self
            .events
            .iter()
            .filter(|(_, stored)| stored.calendar == calendar && !stored.deleted)
            .flat_map(|(id, _)| self.instances(id, after, before))
            .collect();
        let mut body = json!({ "value": entries.iter().skip(offset).take(page_size).cloned().collect::<Vec<_>>() });
        if entries.len() > offset + page_size {
            body["@odata.nextLink"] = json!(format!(
                "{link}?startDateTime={start}&endDateTime={end}&$skip={}",
                offset + page_size
            ));
        }

        View::Page(body)
    }

    /// One event by its id, or by the id of one of its occurrences, with
    /// its description.
    pub fn one(&self, id: &str) -> Option<Value> {
        let own = id.split('.').next().unwrap_or(id);
        let stored = self.events.get(own).filter(|stored| !stored.deleted)?;
        // A series by its own id is its master: as its first occurrence
        // was made, with how it repeats.
        if let (true, Some(series)) = (own == id, &stored.event.series) {
            let first = occurrences(series, Timestamp::MIN, Timestamp::MAX, 1)
                .ok()?
                .into_iter()
                .next()?;
            let mut plain = stored.event.clone();
            plain.series = None;
            plain.start = format!("{}Z", utc(first.start).trim_end_matches(".0000000"));
            plain.end = format!("{}Z", utc(first.end).trim_end_matches(".0000000"));
            let alone = Self {
                events: BTreeMap::from([(
                    own.to_owned(),
                    Stored {
                        calendar: stored.calendar.clone(),
                        event: plain,
                        deleted: false,
                    },
                )]),
                seq: self.seq,
                ..Self::default()
            };
            let mut master = alone
                .instances(own, Timestamp::MIN, Timestamp::MAX)
                .into_iter()
                .next()?;
            master["type"] = json!("seriesMaster");
            master["recurrence"] = series.to_graph(&stored.event.zone);
            master["body"] = json!({ "contentType": "text", "content": stored.event.body.clone().unwrap_or_default() });
            return Some(master);
        }
        let mut found = self
            .instances(
                own,
                Timestamp::UNIX_EPOCH,
                wider(Timestamp::UNIX_EPOCH, 365 * 200),
            )
            .into_iter()
            .find(|each| each["id"] == id)?;
        found["body"] = match &stored.event.body {
            Some(words) => json!({ "contentType": "text", "content": words }),
            None => json!({
                "contentType": "html",
                "content": format!("<p>About <b>{}</b>.</p>", stored.event.subject)
            }),
        };

        Some(found)
    }

    /// Every occurrence there truly is in a calendar between two moments,
    /// as (id, start, end), for a test to hold the client's copy against.
    pub fn truth(
        &self,
        calendar: &str,
        after: &str,
        before: &str,
    ) -> Vec<(String, String, String)> {
        let (Some(after), Some(before)) = (stamp(after), stamp(before)) else {
            return Vec::new();
        };
        let short = |value: &Value| {
            let text = value["dateTime"].as_str().unwrap_or_default();
            format!("{}Z", text.split('.').next().unwrap_or_default())
        };

        let mut all: Vec<(String, String, String)> = self
            .events
            .iter()
            .filter(|(_, stored)| stored.calendar == calendar && !stored.deleted)
            .flat_map(|(id, _)| self.instances(id, after, before))
            .map(|v| {
                (
                    v["id"].as_str().unwrap_or_default().to_owned(),
                    short(&v["start"]),
                    short(&v["end"]),
                )
            })
            .collect();
        all.sort();
        all
    }
}
