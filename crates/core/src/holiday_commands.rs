//! The commands for public holidays.
//!
//! Microsoft 365 has holiday calendars of its own, but documents no way for
//! another program to turn one on. So the days are carried here
//! (`brolga_calendar::holidays`), and put into the mailbox as a calendar of
//! all-day events like any other: every device that shows the account's
//! calendars shows it, and deleting the calendar takes the days with it.
//!
//! The table of days runs a few years ahead. A calendar is filled to two
//! years on from now, and a note of it is kept in the mailbox's settings so
//! that a later sync adds each further year as it comes within reach.

use std::collections::HashSet;
use std::sync::Arc;

use brolga_calendar::holidays;
use brolga_commands::{CommandError, CommandKind, CommandSpec, Registry, RegistryError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::calendar_commands::{failed, handler, kept, not_made, times};
use crate::mailboxes::{Mailbox, Mailboxes};

/// Where the holiday calendars of a mailbox are noted, as JSON.
const NOTED: &str = "holidays";
/// How many years on from this one a calendar is filled to.
const AHEAD: i16 = 2;
/// How a calendar's year is read to see which days it has: so many events
/// a page, and no more pages than this.
const PAGE_SIZE: u32 = 100;
const MOST_PAGES: u32 = 20;

/// A holiday calendar that was made here, and how far it has been filled.
#[derive(Serialize, Deserialize, Clone)]
struct Kept {
    calendar_id: String,
    country: String,
    region: Option<String>,
    /// The last year whose days are all in the calendar.
    through: i16,
}

fn noted(mailbox: &Mailbox) -> Vec<Kept> {
    mailbox
        .db
        .setting(NOTED)
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn note(mailbox: &Mailbox, all: &[Kept]) -> Result<(), CommandError> {
    let text = serde_json::to_string(all).map_err(failed)?;

    mailbox.db.set_setting(NOTED, &text).map_err(failed)
}

/// Note how far one calendar has been filled, among the others.
fn note_one(mailbox: &Mailbox, one: &Kept) -> Result<(), CommandError> {
    let mut all = noted(mailbox);
    all.retain(|each| each.calendar_id != one.calendar_id);
    all.push(one.clone());

    note(mailbox, &all)
}

/// The year it is, here.
fn this_year() -> i16 {
    jiff::Zoned::now().year()
}

/// Put one year's holidays in a calendar, less those already in it, and
/// say how many were put. A day is there already if the calendar has an
/// all-day event of its name on its date, so that a year half done, or one
/// asked for twice, is not doubled.
///
/// What is there already is asked of the server, not taken from this
/// computer's copy. A day can have been made without the answer ever
/// arriving, and the copy of a calendar like this one is read again only
/// every so often, and only for the stretch of time that is kept: going
/// by it would make such a day a second time.
async fn fill(
    mailbox: &Mailbox,
    calendar_id: &str,
    country: &str,
    region: Option<&str>,
    year: i16,
) -> Result<usize, CommandError> {
    let days = holidays::of(country, region, year).unwrap_or_default();
    // A day either side, for an all-day event is kept by where its midnight fell in UTC.
    let window = (
        format!("{:04}-12-30T00:00:00", year - 1),
        format!("{:04}-01-03T00:00:00", year + 1),
    );
    let mut there: HashSet<(String, String)> = HashSet::new();
    let mut link: Option<String> = None;
    // A calendar of holidays has a few dozen days in a year. A list that
    // does not end is a server gone wrong, and is not followed for ever.
    for _ in 0..MOST_PAGES {
        let page = mailbox
            .graph
            .calendar_view(
                calendar_id,
                link.as_deref(),
                (&window.0, &window.1),
                PAGE_SIZE,
            )
            .await
            .map_err(not_made)?;
        for item in page.items {
            if let brolga_graph::DeltaItem::Changed(event) = item
                && let Some(event) = brolga_sync::event_row(event)
                && event.is_all_day
            {
                there.insert((event.start.chars().take(10).collect(), event.subject));
            }
        }
        match page.next {
            brolga_graph::Next::Page(next) => link = Some(next),
            brolga_graph::Next::Done(_) => {
                link = None;
                break;
            }
        }
    }
    if link.is_some() {
        return Err(failed(
            "Microsoft 365 did not finish saying which days the calendar already has.",
        ));
    }

    let mut put = 0;
    for day in days {
        if there.contains(&(day.date.clone(), day.name.clone())) {
            continue;
        }
        let first: jiff::civil::Date = day.date.parse().map_err(failed)?;
        let after = first.tomorrow().map_err(failed)?;
        let (start, end, zone) = times(&first.to_string(), &after.to_string(), None, true)?;
        // Nobody is busy for it, and nothing rings ("Create event", SOURCES.md).
        let event = json!({
            "subject": day.name,
            "start": { "dateTime": start, "timeZone": zone },
            "end": { "dateTime": end, "timeZone": zone },
            "isAllDay": true,
            "showAs": "free",
            "isReminderOn": false,
        });
        let made = mailbox
            .graph
            .create_event(Some(calendar_id), &event)
            .await
            .map_err(not_made)?;
        kept(mailbox, calendar_id, made)?;
        put += 1;
    }

    Ok(put)
}

/// Fill a calendar from the year after the last it has, as far as the
/// table and `AHEAD` reach, noting each year as it is done. Says how many
/// days were put, and what stopped it if anything did.
async fn fill_on(mailbox: &Mailbox, mut one: Kept) -> (usize, Option<CommandError>) {
    let last = holidays::years().1.min(this_year() + AHEAD);
    let mut put = 0;
    while one.through < last {
        let year = one.through + 1;
        match fill(
            mailbox,
            &one.calendar_id,
            &one.country,
            one.region.as_deref(),
            year,
        )
        .await
        {
            Ok(more) => put += more,
            Err(error) => return (put, Some(error)),
        }
        one.through = year;
        if let Err(error) = note_one(mailbox, &one) {
            return (put, Some(error));
        }
    }

    (put, None)
}

/// At a sync: add the years that have come within reach to each holiday
/// calendar, and forget the ones that have been deleted. Trouble is left
/// for the next sync: the days are not urgent.
pub(crate) async fn top_up(mailbox: &Mailbox) {
    let all = noted(mailbox);
    if all.is_empty() {
        return;
    }
    let Ok(calendars) = mailbox.db.calendars() else {
        return;
    };
    let (still, gone): (Vec<Kept>, Vec<Kept>) = all.into_iter().partition(|each| {
        calendars
            .iter()
            .any(|calendar| calendar.id == each.calendar_id)
    });
    if !gone.is_empty() && note(mailbox, &still).is_err() {
        return;
    }
    for one in still {
        let _ = fill_on(mailbox, one).await;
    }
}

async fn places(_: Arc<Mailboxes>, _: Nothing) -> Result<Value, CommandError> {
    let (first_year, last_year) = holidays::years();
    let countries: Vec<Value> = holidays::places()
        .into_iter()
        .map(|place| {
            json!({
                "code": place.code,
                "name": place.name,
                "regions": place.regions.iter().map(|(code, name)| json!({ "code": code, "name": name })).collect::<Vec<_>>(),
            })
        })
        .collect();

    Ok(
        json!({ "countries": countries, "first_year": first_year, "last_year": last_year, "source": holidays::source() }),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nothing {}

#[derive(Deserialize)]
struct Add {
    account_id: Option<String>,
    country: String,
    region: Option<String>,
}

async fn add(mailboxes: Arc<Mailboxes>, args: Add) -> Result<Value, CommandError> {
    let region = args
        .region
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty());
    let place = holidays::name_of(args.country.trim(), region).ok_or_else(|| {
        failed("There are no holidays here for that country, or that region of it. See calendar.holidays.places.")
    })?;
    let account = mailboxes
        .account(args.account_id.as_deref())
        .map_err(failed)?;
    let mailbox = mailboxes.open(&account).await.map_err(failed)?;
    if mailbox.shared.is_some() || mailbox.archive {
        return Err(failed(
            "Holidays go in a calendar of your own: choose your own account, not a mailbox someone shared or an archive.",
        ));
    }
    let name = format!("{place} holidays");
    if mailbox
        .db
        .calendars()
        .map_err(failed)?
        .iter()
        .any(|each| each.name.eq_ignore_ascii_case(&name))
    {
        return Err(failed(format!(
            "There is already a calendar called `{name}`."
        )));
    }

    let made = mailbox.graph.create_calendar(&name).await.map_err(|error| {
        if error.is_transient() {
            failed("Microsoft 365 could not be reached. Sync and look before trying again, in case the calendar was made.")
        } else {
            failed(error)
        }
    })?;
    // The list is read again, so that the new one is here as the server has it.
    let _ = mailbox.calendars.sync_calendars().await;

    // From the start of this year, so that the days just gone are there to look back on.
    let one = Kept {
        calendar_id: made.id.clone(),
        country: args.country.trim().to_uppercase(),
        region: region.map(str::to_uppercase),
        through: this_year().max(holidays::years().0) - 1,
    };
    note_one(&mailbox, &one)?;
    let (added, stopped) = fill_on(&mailbox, one).await;

    Ok(json!({
        "calendar_id": made.id,
        "name": name,
        "added": added,
        "complete": stopped.is_none(),
        "problem": stopped.map(|error| format!("Not every day went in: {error} The rest are added at a later sync.")),
    }))
}

pub(crate) fn register(
    registry: &mut Registry,
    mailboxes: Arc<Mailboxes>,
) -> Result<(), RegistryError> {
    let account_id = json!({ "type": "string", "description": "Which account, from accounts.list. May be left out when there is only one." });

    registry.register(
        CommandSpec {
            id: "calendar.holidays.places".to_owned(),
            title: "Places with holidays".to_owned(),
            description: "The countries whose public holidays can be added as a calendar, each with the regions of it (states, provinces) that keep days of their own, \
                          and the years the days are known for. Nothing is asked of Microsoft 365."
                .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "countries": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "code": { "type": "string", "description": "Two letters: AU." },
                                "name": { "type": "string" },
                                "regions": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": { "code": { "type": "string" }, "name": { "type": "string" } },
                                        "required": ["code", "name"]
                                    }
                                }
                            },
                            "required": ["code", "name", "regions"]
                        }
                    },
                    "first_year": { "type": "integer" },
                    "last_year": { "type": "integer" },
                    "source": { "type": "string", "description": "Where the days came from." }
                },
                "required": ["countries", "first_year", "last_year"]
            }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, places),
    )?;

    registry.register(
        CommandSpec {
            id: "calendar.holidays.add".to_owned(),
            title: "Add a holiday calendar".to_owned(),
            description: "Make a calendar of a country's public holidays, or of one region's, in your own mailbox: all-day events from the start of this year \
                          to two years on, with later years added at a sync as they come within reach. It is an ordinary calendar: hide it or delete it like any other. \
                          It needs Microsoft 365 to be in reach."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id,
                    "country": { "type": "string", "description": "A country's code from calendar.holidays.places: AU." },
                    "region": { "type": "string", "description": "A region's code from the same list: QLD. Left out, the days the whole country keeps." }
                },
                "required": ["country"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "calendar_id": { "type": "string" },
                    "name": { "type": "string" },
                    "added": { "type": "integer", "description": "How many days were put in." },
                    "complete": { "type": "boolean", "description": "False where something stopped it part way: see problem." },
                    "problem": { "type": ["string", "null"] }
                },
                "required": ["calendar_id", "name", "added", "complete"]
            }),
            examples: vec![json!({ "country": "AU", "region": "QLD" })],
        },
        handler(&mailboxes, add),
    )?;

    Ok(())
}
