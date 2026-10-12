//! Calendars and events, as Graph gives them.

use serde::Deserialize;

use crate::mail::{DateTimeTimeZone, EmailAddress, ItemBody, Recipient, or_default};

/// A calendar ("calendar resource type", SOURCES.md).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Calendar {
    pub id: String,
    pub name: Option<String>,
    /// A named colour theme, such as `lightBlue`, or `auto`.
    pub color: Option<String>,
    /// The colour as `#RRGGBB`, where the calendar has one of its own.
    pub hex_color: Option<String>,
    pub is_default_calendar: Option<bool>,
    pub can_edit: Option<bool>,
    pub owner: Option<EmailAddress>,
}

/// An event as a calendar view gives it: one occurrence of a series is an
/// event of its own, with its own id ("event resource type").
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub subject: Option<String>,
    pub body_preview: Option<String>,
    pub body: Option<ItemBody>,
    /// In UTC unless the request asked otherwise.
    pub start: Option<DateTimeTimeZone>,
    pub end: Option<DateTimeTimeZone>,
    pub is_all_day: Option<bool>,
    /// The zone the event was made in, as Windows or the zone database names it.
    pub original_start_time_zone: Option<String>,
    pub original_end_time_zone: Option<String>,
    pub location: Option<Location>,
    /// `free`, `tentative`, `busy`, `oof`, `workingElsewhere` or `unknown`.
    pub show_as: Option<String>,
    /// `normal`, `personal`, `private` or `confidential`.
    pub sensitivity: Option<String>,
    pub is_cancelled: Option<bool>,
    pub is_organizer: Option<bool>,
    pub organizer: Option<Recipient>,
    pub attendees: Option<Vec<Attendee>>,
    pub response_status: Option<ResponseStatus>,
    /// `singleInstance`, `occurrence`, `exception` or `seriesMaster`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub series_master_id: Option<String>,
    pub categories: Option<Vec<String>>,
    pub is_reminder_on: Option<bool>,
    pub reminder_minutes_before_start: Option<i64>,
    pub online_meeting: Option<OnlineMeeting>,
    pub web_link: Option<String>,
    pub change_key: Option<String>,
    /// How a series repeats. Only a series master has it.
    pub recurrence: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attendee {
    /// `required`, `optional` or `resource`.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub email_address: Option<EmailAddress>,
    pub status: Option<ResponseStatus>,
    /// Another time they would rather have, if they proposed one.
    pub proposed_new_time: Option<TimeSlot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeSlot {
    #[serde(default, deserialize_with = "or_default")]
    pub start: DateTimeTimeZone,
    #[serde(default, deserialize_with = "or_default")]
    pub end: DateTimeTimeZone,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseStatus {
    /// `none`, `organizer`, `tentativelyAccepted`, `accepted`, `declined` or `notResponded`.
    pub response: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineMeeting {
    pub join_url: Option<String>,
}
