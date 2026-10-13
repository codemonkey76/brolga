#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The golden cases the build spec asks for: repeating events across the
//! changes of clocks in Australia/Brisbane (which has none),
//! Australia/Sydney, Europe/London, America/New_York and a half-hour zone,
//! Asia/Kolkata.
//!
//! Each expected time here was worked out by hand from the published dates
//! of the clock changes, not by the code under test. They are to be checked
//! against what Microsoft 365's calendarView gives for the same series once
//! there is a test mailbox to ask.

use brolga_calendar::{DayOfWeek, Ends, Pattern, Series, WeekIndex, occurrences};
use jiff::Timestamp;
use jiff::civil::{Date, DateTime, date};

fn series(zone: &str, start: DateTime, minutes: i64, pattern: Pattern, ends: Ends) -> Series {
    Series {
        start,
        end: start
            .checked_add(jiff::Span::new().minutes(minutes))
            .unwrap(),
        zone: zone.to_owned(),
        pattern,
        from: start.date(),
        ends,
    }
}

fn weekly(day: DayOfWeek) -> Pattern {
    Pattern::Weekly {
        interval: 1,
        days_of_week: vec![day],
        first_day_of_week: DayOfWeek::Sunday,
    }
}

/// Every start in a window, as UTC text.
fn starts(series: &Series, after: &str, before: &str) -> Vec<String> {
    let after: Timestamp = after.parse().unwrap();
    let before: Timestamp = before.parse().unwrap();

    occurrences(series, after, before, 1000)
        .unwrap()
        .iter()
        .map(|o| o.start.to_string())
        .collect()
}

fn all(series: &Series) -> Vec<brolga_calendar::Occurrence> {
    occurrences(
        series,
        "2000-01-01T00:00:00Z".parse().unwrap(),
        "2100-01-01T00:00:00Z".parse().unwrap(),
        10_000,
    )
    .unwrap()
}

fn local_days(series: &Series) -> Vec<Date> {
    all(series).iter().map(|o| o.local_start.date()).collect()
}

// ---- the changes of clocks ---------------------------------------------

#[test]
fn brisbane_has_no_change_of_clocks_so_nine_is_always_the_same_moment() {
    let s = series(
        "Australia/Brisbane",
        date(2026, 3, 30).at(9, 0, 0, 0),
        60,
        weekly(DayOfWeek::Monday),
        Ends::After(30),
    );

    // 9 am at UTC+10 is 23:00 the day before, every week of the year.
    assert!(
        all(&s)
            .iter()
            .all(|o| o.start.to_string().ends_with("T23:00:00Z"))
    );
    assert_eq!(all(&s).len(), 30);
}

#[test]
fn sydney_weekly_nine_stays_nine_when_daylight_saving_ends_and_when_it_starts() {
    let s = series(
        "Australia/Sydney",
        date(2026, 3, 23).at(9, 0, 0, 0),
        60,
        weekly(DayOfWeek::Monday),
        Ends::Never,
    );

    // Daylight saving ends on Sunday 5 April 2026: UTC+11 becomes UTC+10.
    assert_eq!(
        starts(&s, "2026-03-22T00:00:00Z", "2026-04-14T00:00:00Z"),
        [
            "2026-03-22T22:00:00Z",
            "2026-03-29T22:00:00Z",
            "2026-04-05T23:00:00Z",
            "2026-04-12T23:00:00Z",
        ]
    );
    // It starts again on Sunday 4 October 2026: UTC+10 becomes UTC+11.
    assert_eq!(
        starts(&s, "2026-09-27T00:00:00Z", "2026-10-13T00:00:00Z"),
        [
            "2026-09-27T23:00:00Z",
            "2026-10-04T22:00:00Z",
            "2026-10-11T22:00:00Z"
        ]
    );
}

#[test]
fn london_weekly_nine_stays_nine_through_summer_time() {
    let s = series(
        "Europe/London",
        date(2026, 3, 16).at(9, 0, 0, 0),
        30,
        weekly(DayOfWeek::Monday),
        Ends::Never,
    );

    // Summer time starts on Sunday 29 March 2026 and ends on Sunday 25 October.
    assert_eq!(
        starts(&s, "2026-03-16T00:00:00Z", "2026-04-07T00:00:00Z"),
        [
            "2026-03-16T09:00:00Z",
            "2026-03-23T09:00:00Z",
            "2026-03-30T08:00:00Z",
            "2026-04-06T08:00:00Z",
        ]
    );
    assert_eq!(
        starts(&s, "2026-10-19T00:00:00Z", "2026-11-03T00:00:00Z"),
        [
            "2026-10-19T08:00:00Z",
            "2026-10-26T09:00:00Z",
            "2026-11-02T09:00:00Z"
        ]
    );
}

#[test]
fn new_york_weekly_nine_stays_nine_through_daylight_time() {
    let s = series(
        "America/New_York",
        date(2026, 3, 2).at(9, 0, 0, 0),
        60,
        weekly(DayOfWeek::Monday),
        Ends::Never,
    );

    // Daylight time starts on Sunday 8 March 2026 and ends on Sunday 1 November.
    assert_eq!(
        starts(&s, "2026-03-02T00:00:00Z", "2026-03-17T00:00:00Z"),
        [
            "2026-03-02T14:00:00Z",
            "2026-03-09T13:00:00Z",
            "2026-03-16T13:00:00Z"
        ]
    );
    assert_eq!(
        starts(&s, "2026-10-26T00:00:00Z", "2026-11-10T00:00:00Z"),
        [
            "2026-10-26T13:00:00Z",
            "2026-11-02T14:00:00Z",
            "2026-11-09T14:00:00Z"
        ]
    );
}

#[test]
fn kolkata_is_half_an_hour_off_the_hour_and_never_changes() {
    let s = series(
        "Asia/Kolkata",
        date(2026, 1, 5).at(9, 0, 0, 0),
        45,
        Pattern::Daily { interval: 1 },
        Ends::After(400),
    );

    // 9 am at UTC+5:30 is 03:30 UTC, on every day of the year.
    let every = all(&s);
    assert_eq!(every.len(), 400);
    assert!(
        every
            .iter()
            .all(|o| o.start.to_string().ends_with("T03:30:00Z"))
    );
    assert!(
        every
            .iter()
            .all(|o| o.end.to_string().ends_with("T04:15:00Z"))
    );
}

#[test]
fn a_meeting_one_place_is_seen_at_its_true_time_from_another_across_both_changes() {
    // A London 4 pm weekly meeting, as someone in Sydney sees it. Both places
    // change their clocks, on different dates and in opposite directions.
    let s = series(
        "Europe/London",
        date(2026, 3, 19).at(16, 0, 0, 0),
        60,
        weekly(DayOfWeek::Thursday),
        Ends::Never,
    );
    let sydney = jiff::tz::TimeZone::get("Australia/Sydney").unwrap();
    let seen: Vec<String> = occurrences(
        &s,
        "2026-03-19T00:00:00Z".parse().unwrap(),
        "2026-04-10T00:00:00Z".parse().unwrap(),
        10,
    )
    .unwrap()
    .iter()
    .map(|o| o.start.to_zoned(sydney.clone()).datetime().to_string())
    .collect();

    assert_eq!(
        seen,
        [
            // London on GMT, Sydney on daylight time: eleven hours apart.
            "2026-03-20T03:00:00",
            "2026-03-27T03:00:00",
            // London has gone to summer time: ten hours.
            "2026-04-03T02:00:00",
            // Sydney has left daylight time: nine hours.
            "2026-04-10T01:00:00",
        ]
    );
}

#[test]
fn a_time_the_clocks_skip_is_taken_as_the_time_they_jump_to() {
    // In Sydney on 4 October 2026, 2:00 am becomes 3:00 am: there is no 2:30.
    let s = series(
        "Australia/Sydney",
        date(2026, 10, 3).at(2, 30, 0, 0),
        30,
        Pattern::Daily { interval: 1 },
        Ends::After(3),
    );

    assert_eq!(
        starts(&s, "2026-10-01T00:00:00Z", "2026-10-10T00:00:00Z"),
        [
            "2026-10-02T16:30:00Z", // 2:30 am at UTC+10
            "2026-10-03T16:30:00Z", // 3:30 am at UTC+11: the half hour after the jump
            "2026-10-04T15:30:00Z", // 2:30 am at UTC+11
        ]
    );
}

#[test]
fn a_time_that_comes_twice_is_taken_as_the_first_of_the_two() {
    // In Sydney on 5 April 2026, 3:00 am becomes 2:00 am: 2:30 happens twice.
    let s = series(
        "Australia/Sydney",
        date(2026, 4, 4).at(2, 30, 0, 0),
        30,
        Pattern::Daily { interval: 1 },
        Ends::After(3),
    );

    assert_eq!(
        starts(&s, "2026-04-01T00:00:00Z", "2026-04-10T00:00:00Z"),
        [
            "2026-04-03T15:30:00Z", // 2:30 am at UTC+11
            "2026-04-04T15:30:00Z", // the first 2:30 am, still at UTC+11
            "2026-04-05T16:30:00Z", // 2:30 am at UTC+10
        ]
    );
}

#[test]
fn a_meeting_is_as_long_on_the_day_the_clocks_change_as_on_any_other() {
    // A two-hour meeting from 1 am in London on the night summer time starts.
    let s = series(
        "Europe/London",
        date(2026, 3, 28).at(1, 0, 0, 0),
        120,
        Pattern::Daily { interval: 1 },
        Ends::After(3),
    );

    for occurrence in all(&s) {
        assert_eq!(
            occurrence.end.duration_since(occurrence.start).as_secs(),
            7200
        );
    }
}

// ---- the patterns --------------------------------------------------------

#[test]
fn daily_every_third_day() {
    let s = series(
        "Australia/Brisbane",
        date(2026, 1, 30).at(8, 0, 0, 0),
        15,
        Pattern::Daily { interval: 3 },
        Ends::After(4),
    );

    assert_eq!(
        local_days(&s),
        [
            date(2026, 1, 30),
            date(2026, 2, 2),
            date(2026, 2, 5),
            date(2026, 2, 8)
        ]
    );
}

#[test]
fn weekly_on_several_days_every_other_week_from_the_middle_of_a_week() {
    // Starts on a Wednesday; the Monday of that first week is before the start.
    let s = series(
        "Australia/Brisbane",
        date(2026, 10, 7).at(10, 0, 0, 0),
        60,
        Pattern::Weekly {
            interval: 2,
            days_of_week: vec![DayOfWeek::Friday, DayOfWeek::Monday, DayOfWeek::Wednesday],
            first_day_of_week: DayOfWeek::Sunday,
        },
        Ends::After(6),
    );

    assert_eq!(
        local_days(&s),
        [
            date(2026, 10, 7),
            date(2026, 10, 9),
            // A week is skipped, then Monday, Wednesday, Friday.
            date(2026, 10, 19),
            date(2026, 10, 21),
            date(2026, 10, 23),
            date(2026, 11, 2),
        ]
    );
}

#[test]
fn which_day_a_week_begins_on_decides_which_weeks_an_every_other_week_series_is_in() {
    // Starts on a Sunday, on Sundays and Saturdays, every other week.
    let on = |first_day_of_week| {
        local_days(&series(
            "Australia/Brisbane",
            date(2026, 10, 4).at(10, 0, 0, 0),
            60,
            Pattern::Weekly {
                interval: 2,
                days_of_week: vec![DayOfWeek::Sunday, DayOfWeek::Saturday],
                first_day_of_week,
            },
            Ends::After(4),
        ))
    };

    // A week beginning on Sunday holds that Sunday and the Saturday after it.
    assert_eq!(
        on(DayOfWeek::Sunday),
        [
            date(2026, 10, 4),
            date(2026, 10, 10),
            date(2026, 10, 18),
            date(2026, 10, 24)
        ]
    );
    // A week beginning on Monday ends with that Sunday: the Saturday before
    // it is before the start, and the next of either is a fortnight on.
    assert_eq!(
        on(DayOfWeek::Monday),
        [
            date(2026, 10, 4),
            date(2026, 10, 17),
            date(2026, 10, 18),
            date(2026, 10, 31)
        ]
    );
}

#[test]
fn monthly_on_the_31st_falls_on_the_last_day_of_a_shorter_month() {
    let s = series(
        "Australia/Brisbane",
        date(2027, 12, 31).at(9, 0, 0, 0),
        60,
        Pattern::AbsoluteMonthly {
            interval: 1,
            day_of_month: 31,
        },
        Ends::After(5),
    );

    assert_eq!(
        local_days(&s),
        [
            date(2027, 12, 31),
            date(2028, 1, 31),
            date(2028, 2, 29), // a leap year
            date(2028, 3, 31),
            date(2028, 4, 30),
        ]
    );
}

#[test]
fn monthly_on_the_second_tuesday_and_on_the_last_friday() {
    let second_tuesday = series(
        "Australia/Brisbane",
        date(2026, 10, 13).at(14, 0, 0, 0),
        60,
        Pattern::RelativeMonthly {
            interval: 1,
            days_of_week: vec![DayOfWeek::Tuesday],
            index: WeekIndex::Second,
        },
        Ends::After(3),
    );
    let last_friday = series(
        "Australia/Brisbane",
        date(2026, 10, 30).at(14, 0, 0, 0),
        60,
        Pattern::RelativeMonthly {
            interval: 2,
            days_of_week: vec![DayOfWeek::Friday],
            index: WeekIndex::Last,
        },
        Ends::After(3),
    );

    assert_eq!(
        local_days(&second_tuesday),
        [date(2026, 10, 13), date(2026, 11, 10), date(2026, 12, 8)]
    );
    assert_eq!(
        local_days(&last_friday),
        [date(2026, 10, 30), date(2026, 12, 25), date(2027, 2, 26)]
    );
}

#[test]
fn the_last_weekday_of_the_month_is_the_last_of_any_of_the_days_named() {
    let weekdays = vec![
        DayOfWeek::Monday,
        DayOfWeek::Tuesday,
        DayOfWeek::Wednesday,
        DayOfWeek::Thursday,
        DayOfWeek::Friday,
    ];
    let s = series(
        "Australia/Brisbane",
        date(2026, 10, 30).at(17, 0, 0, 0),
        30,
        Pattern::RelativeMonthly {
            interval: 1,
            days_of_week: weekdays,
            index: WeekIndex::Last,
        },
        Ends::After(4),
    );

    assert_eq!(
        local_days(&s),
        [
            date(2026, 10, 30), // a Friday
            date(2026, 11, 30), // a Monday
            date(2026, 12, 31), // a Thursday
            date(2027, 1, 29),  // a Friday: the 30th and 31st are the weekend
        ]
    );
}

#[test]
fn yearly_on_the_29th_of_february_falls_on_the_28th_in_other_years() {
    let s = series(
        "Australia/Brisbane",
        date(2028, 2, 29).at(9, 0, 0, 0),
        60,
        Pattern::AbsoluteYearly {
            interval: 1,
            month: 2,
            day_of_month: 29,
        },
        Ends::After(5),
    );

    assert_eq!(
        local_days(&s),
        [
            date(2028, 2, 29),
            date(2029, 2, 28),
            date(2030, 2, 28),
            date(2031, 2, 28),
            date(2032, 2, 29),
        ]
    );
}

#[test]
fn yearly_on_the_first_tuesday_of_november() {
    let s = series(
        "Australia/Sydney",
        date(2026, 11, 3).at(15, 0, 0, 0),
        10,
        Pattern::RelativeYearly {
            interval: 1,
            month: 11,
            days_of_week: vec![DayOfWeek::Tuesday],
            index: WeekIndex::First,
        },
        Ends::After(3),
    );

    assert_eq!(
        local_days(&s),
        [date(2026, 11, 3), date(2027, 11, 2), date(2028, 11, 7)]
    );
}

// ---- where a series ends, and what is asked of it -----------------------

#[test]
fn a_series_that_ends_on_a_day_has_its_last_occurrence_on_that_day_if_one_falls_there() {
    let until = |last: Date| {
        local_days(&series(
            "Australia/Sydney",
            date(2026, 10, 5).at(9, 0, 0, 0),
            60,
            weekly(DayOfWeek::Monday),
            Ends::On(last),
        ))
    };

    assert_eq!(
        until(date(2026, 10, 19)),
        [date(2026, 10, 5), date(2026, 10, 12), date(2026, 10, 19)]
    );
    assert_eq!(
        until(date(2026, 10, 18)),
        [date(2026, 10, 5), date(2026, 10, 12)]
    );
}

#[test]
fn the_number_of_occurrences_counts_from_the_first_not_from_what_was_asked_for() {
    let s = series(
        "Australia/Brisbane",
        date(2026, 1, 1).at(9, 0, 0, 0),
        60,
        Pattern::Daily { interval: 1 },
        Ends::After(10),
    );

    // The last three of the ten, and nothing after them.
    assert_eq!(
        starts(&s, "2026-01-07T00:00:00Z", "2027-01-01T00:00:00Z"),
        [
            "2026-01-07T23:00:00Z",
            "2026-01-08T23:00:00Z",
            "2026-01-09T23:00:00Z"
        ]
    );
}

#[test]
fn only_what_overlaps_the_time_asked_about_is_given_and_no_more_than_asked_for() {
    let s = series(
        "Australia/Brisbane",
        date(2026, 1, 1).at(9, 0, 0, 0),
        60,
        Pattern::Daily { interval: 1 },
        Ends::Never,
    );
    let window = |after: &str, before: &str, limit: usize| {
        occurrences(&s, after.parse().unwrap(), before.parse().unwrap(), limit)
            .unwrap()
            .len()
    };

    // One that has begun and not ended is in; one that ends as the time asked about begins is not.
    assert_eq!(
        window("2026-01-01T23:30:00Z", "2026-01-02T00:00:00Z", 100),
        1
    );
    assert_eq!(
        window("2026-01-02T00:00:00Z", "2026-01-02T22:00:00Z", 100),
        0
    );
    assert_eq!(
        window("2026-01-01T00:00:00Z", "2030-01-01T00:00:00Z", 25),
        25
    );
    // A series without end is not walked for ever.
    assert!(window("2026-01-01T00:00:00Z", "9000-01-01T00:00:00Z", usize::MAX) <= 20_000);
}

#[test]
fn a_series_that_cannot_be_is_refused_not_guessed_at() {
    let base = series(
        "Australia/Brisbane",
        date(2026, 1, 1).at(9, 0, 0, 0),
        60,
        Pattern::Daily { interval: 1 },
        Ends::Never,
    );
    let ask = |s: &Series| {
        occurrences(
            s,
            Timestamp::UNIX_EPOCH,
            "2030-01-01T00:00:00Z".parse().unwrap(),
            10,
        )
    };

    let mut nowhere = base.clone();
    nowhere.zone = "Mars/Olympus Mons".to_owned();
    assert!(ask(&nowhere).is_err());

    for pattern in [
        Pattern::Daily { interval: 0 },
        Pattern::Weekly {
            interval: 1,
            days_of_week: vec![],
            first_day_of_week: DayOfWeek::Sunday,
        },
        Pattern::AbsoluteYearly {
            interval: 1,
            month: 13,
            day_of_month: 1,
        },
        Pattern::AbsoluteMonthly {
            interval: 1,
            day_of_month: 0,
        },
    ] {
        let mut impossible = base.clone();
        impossible.pattern = pattern;
        assert!(ask(&impossible).is_err(), "{:?}", impossible.pattern);
    }
}

#[test]
fn occurrences_come_in_order_with_none_twice_whatever_the_pattern() {
    let patterns = [
        Pattern::Daily { interval: 2 },
        Pattern::Weekly {
            interval: 3,
            days_of_week: vec![DayOfWeek::Saturday, DayOfWeek::Sunday, DayOfWeek::Sunday],
            first_day_of_week: DayOfWeek::Wednesday,
        },
        Pattern::AbsoluteMonthly {
            interval: 5,
            day_of_month: 30,
        },
        Pattern::RelativeMonthly {
            interval: 1,
            days_of_week: vec![DayOfWeek::Monday, DayOfWeek::Friday],
            index: WeekIndex::Fourth,
        },
        Pattern::RelativeYearly {
            interval: 2,
            month: 2,
            days_of_week: vec![DayOfWeek::Sunday],
            index: WeekIndex::Last,
        },
    ];

    for zone in [
        "Australia/Sydney",
        "Europe/London",
        "America/New_York",
        "Asia/Kolkata",
    ] {
        for pattern in &patterns {
            let s = series(
                zone,
                date(2026, 1, 31).at(2, 30, 0, 0),
                90,
                pattern.clone(),
                Ends::After(200),
            );
            let every = all(&s);
            assert!(!every.is_empty(), "{zone} {pattern:?}");
            assert!(
                every.windows(2).all(|pair| pair[0].start < pair[1].start),
                "{zone} {pattern:?}"
            );
            assert!(
                every
                    .iter()
                    .all(|o| o.local_start.date() >= date(2026, 1, 31))
            );
        }
    }
}

#[test]
fn a_series_that_steps_past_the_last_year_there_is_ends_and_does_not_panic() {
    // Found by the fuzzer: every 9991 years from 2020 is the year 12011.
    for (kind, extra) in [
        (
            "relativeYearly",
            r#""daysOfWeek":["friday"],"index":"last","month":10"#,
        ),
        ("absoluteYearly", r#""dayOfMonth":29,"month":2"#),
    ] {
        let recurrence: serde_json::Value = serde_json::from_str(&format!(
            r#"{{"pattern":{{"type":"{kind}","interval":9991,{extra}}},"range":{{"type":"noEnd","startDate":"2020-02-29"}}}}"#
        ))
        .unwrap();
        let s = Series::from_graph(
            date(2020, 2, 29).at(0, 0, 0, 0),
            date(2020, 3, 1).at(0, 0, 0, 0),
            "America/New_York",
            &recurrence,
        )
        .unwrap();

        let found = occurrences(
            &s,
            "2020-01-01T00:00:00Z".parse().unwrap(),
            "2032-01-01T00:00:00Z".parse().unwrap(),
            500,
        );
        assert_eq!(found.map(|found| found.len()).ok(), Some(1), "{kind}");
    }
}
