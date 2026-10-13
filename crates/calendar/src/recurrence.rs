//! When a repeating event falls.
//!
//! A series is expanded in its own time zone and only then turned into
//! moments in time. "Every Monday at nine" means nine by the clock on the
//! wall where the meeting is, so across a change of clocks its moment in
//! UTC moves by an hour and its time on the wall does not.
//!
//! The pattern and the range are Microsoft Graph's ("recurrencePattern" and
//! "recurrenceRange resource types", SOURCES.md). Where the server and this
//! disagree the server is right, and the disagreement is a bug here.

use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, ToSpan};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DayOfWeek {
    Sunday,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
}

impl DayOfWeek {
    fn weekday(self) -> Weekday {
        match self {
            Self::Sunday => Weekday::Sunday,
            Self::Monday => Weekday::Monday,
            Self::Tuesday => Weekday::Tuesday,
            Self::Wednesday => Weekday::Wednesday,
            Self::Thursday => Weekday::Thursday,
            Self::Friday => Weekday::Friday,
            Self::Saturday => Weekday::Saturday,
        }
    }
}

/// Which of the days in a month a relative pattern means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WeekIndex {
    First,
    Second,
    Third,
    Fourth,
    Last,
}

/// How an event repeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Pattern {
    /// Every `interval` days.
    Daily { interval: u32 },
    /// On these days, every `interval` weeks, a week beginning on `first_day_of_week`.
    #[serde(rename_all = "camelCase")]
    Weekly {
        interval: u32,
        days_of_week: Vec<DayOfWeek>,
        first_day_of_week: DayOfWeek,
    },
    /// On this day of the month, every `interval` months.
    #[serde(rename_all = "camelCase")]
    AbsoluteMonthly { interval: u32, day_of_month: u8 },
    /// On the first, second … or last of these days in the month, every `interval` months.
    #[serde(rename_all = "camelCase")]
    RelativeMonthly {
        interval: u32,
        days_of_week: Vec<DayOfWeek>,
        index: WeekIndex,
    },
    /// On this day of this month, every `interval` years.
    #[serde(rename_all = "camelCase")]
    AbsoluteYearly {
        interval: u32,
        month: u8,
        day_of_month: u8,
    },
    /// On the first, second … or last of these days in this month, every `interval` years.
    #[serde(rename_all = "camelCase")]
    RelativeYearly {
        interval: u32,
        month: u8,
        days_of_week: Vec<DayOfWeek>,
        index: WeekIndex,
    },
}

/// When a series stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ends {
    Never,
    /// On this day, which may itself have an occurrence.
    On(Date),
    /// After this many occurrences.
    After(u32),
}

/// A repeating event: its first occurrence, how it repeats and when it stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Series {
    /// The start of the first occurrence, by the clock in `zone`.
    pub start: DateTime,
    /// The end of the first occurrence, by the clock in `zone`.
    pub end: DateTime,
    /// The zone the event is in, as the zone database names it.
    pub zone: String,
    pub pattern: Pattern,
    /// The day the pattern is applied from. The first occurrence is on this
    /// day or the first after it that the pattern allows.
    pub from: Date,
    pub ends: Ends,
}

/// One time a repeating event falls.
impl Series {
    /// A series from how Graph writes one ("patternedRecurrence resource
    /// type", SOURCES.md): its first start and end by the clock in its
    /// zone, and the `recurrence` of its event. Nothing if that cannot be read.
    pub fn from_graph(
        start: DateTime,
        end: DateTime,
        zone: &str,
        recurrence: &serde_json::Value,
    ) -> Option<Self> {
        let pattern: Pattern = serde_json::from_value(recurrence["pattern"].clone()).ok()?;
        let range = &recurrence["range"];
        let from: Date = range["startDate"].as_str()?.parse().ok()?;
        let ends = match range["type"].as_str()? {
            "endDate" => Ends::On(range["endDate"].as_str()?.parse().ok()?),
            "numbered" => Ends::After(u32::try_from(range["numberOfOccurrences"].as_u64()?).ok()?),
            _ => Ends::Never,
        };

        Some(Self {
            start,
            end,
            zone: zone.to_owned(),
            pattern,
            from,
            ends,
        })
    }

    /// The series as Graph writes one, with its zone named as given.
    pub fn to_graph(&self, zone: &str) -> serde_json::Value {
        let mut range = serde_json::json!({
            "type": "noEnd",
            "startDate": self.from.to_string(),
            "endDate": "0001-01-01",
            "numberOfOccurrences": 0,
            "recurrenceTimeZone": zone
        });
        match self.ends {
            Ends::Never => {}
            Ends::On(last) => {
                range["type"] = "endDate".into();
                range["endDate"] = last.to_string().into();
            }
            Ends::After(count) => {
                range["type"] = "numbered".into();
                range["numberOfOccurrences"] = count.into();
            }
        }

        serde_json::json!({ "pattern": self.pattern, "range": range })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occurrence {
    pub start: Timestamp,
    pub end: Timestamp,
    /// The start by the clock in the series' own zone: what identifies the
    /// occurrence, for an exception to be matched to.
    pub local_start: DateTime,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RecurrenceError {
    #[error("`{0}` is not a time zone this client knows")]
    UnknownZone(String),
    #[error("a series must repeat at an interval of at least one")]
    NoInterval,
    #[error("a weekly or relative series must name at least one day of the week")]
    NoDays,
    #[error("there is no month {0}, or no day {1} in any month")]
    NoSuchDay(u8, u8),
    #[error("the series' dates are out of the range of dates there are")]
    OutOfRange,
}

/// The most occurrences ever worked through for one series. A daily event
/// with no end runs for ever; nothing asks for more than a few years of it.
const MOST: usize = 20_000;

/// The occurrences of a series that overlap the time from `after` up to
/// `before`, in order, and never more than `limit` of them.
pub fn occurrences(
    series: &Series,
    after: Timestamp,
    before: Timestamp,
    limit: usize,
) -> Result<Vec<Occurrence>, RecurrenceError> {
    let zone = TimeZone::get(&series.zone)
        .map_err(|_| RecurrenceError::UnknownZone(series.zone.clone()))?;
    validate(&series.pattern)?;

    // How long the event runs, as time that passes: an hour's meeting is an
    // hour long on the day the clocks change too.
    let first_start = moment(&zone, series.start)?;
    let first_end = moment(&zone, series.end)?;
    let length = first_end.duration_since(first_start);
    let time = series.start.time();

    let mut found = Vec::new();
    for (count, day) in days(series).enumerate() {
        // The days come in order, so one past the last date there is comes
        // after any time asked about: the series has run out, not failed.
        let day = match day {
            Err(RecurrenceError::OutOfRange) => break,
            day => day?,
        };
        if count >= MOST {
            break;
        }
        match series.ends {
            Ends::On(last) if day > last => break,
            Ends::After(most) if count as u64 >= u64::from(most) => break,
            _ => {}
        }

        let local_start = day.to_datetime(time);
        let start = moment(&zone, local_start)?;
        if start >= before {
            break;
        }
        let end = start
            .checked_add(length)
            .map_err(|_| RecurrenceError::OutOfRange)?;
        // An occurrence that has ended by `after` is before the time asked
        // about. One of no length at all belongs to the moment it is at.
        if end > after || start >= after {
            found.push(Occurrence {
                start,
                end,
                local_start,
            });
            if found.len() >= limit {
                break;
            }
        }
    }

    Ok(found)
}

/// The moment a time on the wall is, in a zone. A time the clocks skip
/// over (the spring change) is taken as the time they jump to; a time that
/// comes twice (the autumn change) is taken as the first of the two.
fn moment(zone: &TimeZone, local: DateTime) -> Result<Timestamp, RecurrenceError> {
    zone.to_ambiguous_zoned(local)
        .compatible()
        .map(|zoned| zoned.timestamp())
        .map_err(|_| RecurrenceError::OutOfRange)
}

fn validate(pattern: &Pattern) -> Result<(), RecurrenceError> {
    let (interval, days, month, day_of_month) = match pattern {
        Pattern::Daily { interval } => (*interval, None, None, None),
        Pattern::Weekly {
            interval,
            days_of_week,
            ..
        } => (*interval, Some(days_of_week), None, None),
        Pattern::AbsoluteMonthly {
            interval,
            day_of_month,
        } => (*interval, None, None, Some(*day_of_month)),
        Pattern::RelativeMonthly {
            interval,
            days_of_week,
            ..
        } => (*interval, Some(days_of_week), None, None),
        Pattern::AbsoluteYearly {
            interval,
            month,
            day_of_month,
        } => (*interval, None, Some(*month), Some(*day_of_month)),
        Pattern::RelativeYearly {
            interval,
            month,
            days_of_week,
            ..
        } => (*interval, Some(days_of_week), Some(*month), None),
    };

    if interval == 0 {
        return Err(RecurrenceError::NoInterval);
    }
    if days.is_some_and(Vec::is_empty) {
        return Err(RecurrenceError::NoDays);
    }
    let month_ok = month.is_none_or(|m| (1..=12).contains(&m));
    let day_ok = day_of_month.is_none_or(|d| (1..=31).contains(&d));
    if !month_ok || !day_ok {
        return Err(RecurrenceError::NoSuchDay(
            month.unwrap_or(0),
            day_of_month.unwrap_or(0),
        ));
    }

    Ok(())
}

/// The days a series falls on, from its first on, without end.
fn days(series: &Series) -> Box<dyn Iterator<Item = Result<Date, RecurrenceError>> + '_> {
    let from = series.from;
    let step = |n: i64, unit: fn(i64) -> Span| move |k: i64| unit(k * n);

    match &series.pattern {
        Pattern::Daily { interval } => {
            let every = step(i64::from(*interval), |n| n.days());
            Box::new((0i64..).map(move |k| add(from, every(k))))
        }
        Pattern::Weekly {
            interval,
            days_of_week,
            first_day_of_week,
        } => {
            // The week the series starts in begins on the first day of the
            // week on or before its start. Weeks are counted from there.
            let back = from.weekday().since(first_day_of_week.weekday());
            let week_start = add(from, (-i64::from(back)).days());
            let mut in_week: Vec<i64> = days_of_week
                .iter()
                .map(|day| i64::from(day.weekday().since(first_day_of_week.weekday())))
                .collect();
            in_week.sort_unstable();
            in_week.dedup();
            let weeks = i64::from(*interval);

            Box::new(
                (0i64..)
                    .flat_map(move |k| {
                        let week_start = week_start.clone();
                        in_week.clone().into_iter().map(move |offset| {
                            week_start
                                .clone()
                                .and_then(|start| add(start, (k * weeks * 7 + offset).days()))
                        })
                    })
                    // The days of the first week that come before the series starts are not in it.
                    .filter(move |day| day.as_ref().map_or(true, |day| *day >= from)),
            )
        }
        Pattern::AbsoluteMonthly {
            interval,
            day_of_month,
        } => {
            let (interval, day_of_month) = (i64::from(*interval), *day_of_month);
            Box::new(
                (0i64..)
                    .map(move |k| {
                        month_of(from, k * interval)
                            .map(|(year, month)| clamped(year, month, day_of_month))
                    })
                    .filter(move |day| day.as_ref().map_or(true, |day| *day >= from)),
            )
        }
        Pattern::RelativeMonthly {
            interval,
            days_of_week,
            index,
        } => {
            let interval = i64::from(*interval);
            Box::new(
                (0i64..)
                    .map(move |k| {
                        month_of(from, k * interval)
                            .map(|(year, month)| relative(year, month, days_of_week, *index))
                    })
                    .filter_map(Result::transpose)
                    .filter(move |day| day.as_ref().map_or(true, |day| *day >= from)),
            )
        }
        Pattern::AbsoluteYearly {
            interval,
            month,
            day_of_month,
        } => {
            let (interval, month, day_of_month) = (i64::from(*interval), *month, *day_of_month);
            Box::new(
                (0i64..)
                    .map(move |k| {
                        year_of(from, k * interval)
                            .map(|year| clamped(year, month as i8, day_of_month))
                    })
                    .filter(move |day| day.as_ref().map_or(true, |day| *day >= from)),
            )
        }
        Pattern::RelativeYearly {
            interval,
            month,
            days_of_week,
            index,
        } => {
            let (interval, month) = (i64::from(*interval), *month as i8);
            Box::new(
                (0i64..)
                    .map(move |k| {
                        year_of(from, k * interval)
                            .map(|year| relative(year, month, days_of_week, *index))
                    })
                    .filter_map(Result::transpose)
                    .filter(move |day| day.as_ref().map_or(true, |day| *day >= from)),
            )
        }
    }
}

fn add(day: Date, span: Span) -> Result<Date, RecurrenceError> {
    day.checked_add(span)
        .map_err(|_| RecurrenceError::OutOfRange)
}

/// The year and month that is `months` months after the month of `from`.
fn month_of(from: Date, months: i64) -> Result<(i16, i8), RecurrenceError> {
    let first = from.first_of_month();
    let then = add(first, months.months())?;

    Ok((then.year(), then.month()))
}

fn year_of(from: Date, years: i64) -> Result<i16, RecurrenceError> {
    // A year can be counted that there is no date in: the last is 9999.
    i16::try_from(i64::from(from.year()) + years)
        .ok()
        .filter(|year| Date::new(*year, 1, 1).is_ok())
        .ok_or(RecurrenceError::OutOfRange)
}

/// A day of a month, or the month's last day where it has no such day: the
/// 31st of a month of thirty days is its 30th, and the 29th of February in
/// a year without one is the 28th.
fn clamped(year: i16, month: i8, day_of_month: u8) -> Date {
    let first = jiff::civil::date(year, month, 1);
    let day = (day_of_month as i8).clamp(1, first.days_in_month());

    jiff::civil::date(year, month, day)
}

/// The first, second … or last day of a month that is one of `days`.
/// Nothing if the month has no such day, as it has no fifth Monday when
/// "fourth" is the most that can be asked for and is always there; kept for
/// a pattern that names no days.
fn relative(year: i16, month: i8, days: &[DayOfWeek], index: WeekIndex) -> Option<Date> {
    let first = jiff::civil::date(year, month, 1);
    let matching: Vec<Date> = (1..=first.days_in_month())
        .map(|day| jiff::civil::date(year, month, day))
        .filter(|date| days.iter().any(|day| day.weekday() == date.weekday()))
        .collect();

    match index {
        WeekIndex::First => matching.first().copied(),
        WeekIndex::Second => matching.get(1).copied(),
        WeekIndex::Third => matching.get(2).copied(),
        WeekIndex::Fourth => matching.get(3).copied(),
        WeekIndex::Last => matching.last().copied(),
    }
}
