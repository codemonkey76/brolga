//! Which time zone a name means.
//!
//! Microsoft 365 names zones as Windows does ("AUS Eastern Standard Time").
//! The zone database the rest of the world keeps names them otherwise
//! ("Australia/Sydney"). Unicode's CLDR project publishes the table between
//! the two, and it is shipped here as the data it is: to take up a new
//! zone, the file is replaced and nothing else changes.

use std::collections::HashMap;
use std::sync::OnceLock;

/// CLDR's `windowsZones.xml`, as published (SOURCES.md).
const TABLE: &str = include_str!("../data/windowsZones.xml");

/// The value of an attribute in one line of the table.
fn attribute<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let start = line.find(&format!("{name}=\""))? + name.len() + 2;
    let length = line[start..].find('"')?;

    Some(&line[start..start + length])
}

/// Windows' name for a zone, in lower case, to the zone it means. Each
/// Windows zone has one entry for the world at large (territory `001`),
/// which is the one taken.
fn table() -> &'static HashMap<String, &'static str> {
    static PARSED: OnceLock<HashMap<String, &'static str>> = OnceLock::new();

    PARSED.get_or_init(|| {
        TABLE
            .lines()
            .filter(|line| line.contains("<mapZone") && attribute(line, "territory") == Some("001"))
            .filter_map(|line| {
                let windows = attribute(line, "other")?;
                let iana = attribute(line, "type")?.split(' ').next()?;
                Some((windows.to_lowercase(), iana))
            })
            .collect()
    })
}

/// The zone a name means, as the zone database calls it. The name may be
/// Windows', or already the database's own, or one of the ways Microsoft
/// 365 writes UTC. Nothing if it is none of these.
pub fn iana_zone(name: &str) -> Option<&str> {
    let name = name.trim();
    if name.eq_ignore_ascii_case("utc") || name.eq_ignore_ascii_case("tzone://Microsoft/Utc") {
        return Some("UTC");
    }
    if let Some(known) = table().get(&name.to_lowercase()) {
        return Some(known);
    }

    // Already a name the database knows.
    jiff::tz::TimeZone::get(name).ok().map(|_| name)
}

/// Every zone the table names, in lower case, to Windows' name for it.
fn back() -> &'static HashMap<String, &'static str> {
    static PARSED: OnceLock<HashMap<String, &'static str>> = OnceLock::new();

    PARSED.get_or_init(|| {
        let mut all = HashMap::new();
        for line in TABLE.lines().filter(|line| line.contains("<mapZone")) {
            let (Some(windows), Some(zones)) = (attribute(line, "other"), attribute(line, "type"))
            else {
                continue;
            };
            for zone in zones.split(' ').filter(|zone| !zone.is_empty()) {
                all.entry(zone.to_lowercase()).or_insert(windows);
            }
        }
        all
    })
}

/// Windows' name for a zone, which is the one Microsoft 365 is sure to
/// take when an event is made ("dateTimeTimeZone resource type",
/// SOURCES.md). The name may be the zone database's or already Windows'.
/// Nothing if the table has no such zone.
pub fn windows_zone(name: &str) -> Option<&'static str> {
    let name = name.trim().to_lowercase();
    if name == "utc" || name == "etc/utc" || name == "tzone://microsoft/utc" {
        return Some("UTC");
    }
    if let Some((_, windows)) = back()
        .iter()
        .find(|(_, windows)| windows.to_lowercase() == name)
    {
        return Some(windows);
    }

    back().get(&name).copied()
}

/// A date and time as Graph writes one, with the zone it says it is in,
/// as that moment in UTC: `2026-10-20T03:00:00Z`. Nothing if the time or
/// the zone cannot be read.
pub fn utc(date_time: &str, zone: &str) -> Option<String> {
    // Graph writes seven places of seconds; none are kept.
    let whole = date_time.split('.').next()?;
    let local: jiff::civil::DateTime = whole.parse().ok()?;
    let zone = jiff::tz::TimeZone::get(iana_zone(zone)?).ok()?;
    let moment = zone
        .to_ambiguous_zoned(local)
        .compatible()
        .ok()?
        .timestamp();

    Some(moment.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zone_is_named_as_windows_names_it() {
        for (given, windows) in [
            ("Australia/Brisbane", "E. Australia Standard Time"),
            ("Australia/Sydney", "AUS Eastern Standard Time"),
            // Not the zone for the world at large, but one the table has.
            ("Australia/Melbourne", "AUS Eastern Standard Time"),
            ("Europe/London", "GMT Standard Time"),
            ("america/new_york", "Eastern Standard Time"),
            ("Asia/Calcutta", "India Standard Time"),
            ("UTC", "UTC"),
            ("aus eastern standard time", "AUS Eastern Standard Time"),
        ] {
            assert_eq!(windows_zone(given), Some(windows), "{given}");
        }
        assert_eq!(windows_zone("Mars/Olympus_Mons"), None);
    }

    #[test]
    fn windows_names_mean_the_zones_the_table_says() {
        for (windows, iana) in [
            ("E. Australia Standard Time", "Australia/Brisbane"),
            ("AUS Eastern Standard Time", "Australia/Sydney"),
            ("GMT Standard Time", "Europe/London"),
            ("Eastern Standard Time", "America/New_York"),
            // The table keeps the older name, which the database holds as
            // another name for Asia/Kolkata.
            ("India Standard Time", "Asia/Calcutta"),
            ("aus eastern standard time", "Australia/Sydney"),
        ] {
            assert_eq!(iana_zone(windows), Some(iana), "{windows}");
        }
    }

    #[test]
    fn utc_is_known_however_it_is_written_and_a_database_name_passes_through() {
        assert_eq!(iana_zone("UTC"), Some("UTC"));
        assert_eq!(iana_zone("tzone://Microsoft/Utc"), Some("UTC"));
        assert_eq!(iana_zone("Australia/Brisbane"), Some("Australia/Brisbane"));
        assert_eq!(iana_zone("Mars/Olympus Mons"), None);
        assert_eq!(iana_zone(""), None);
    }

    #[test]
    fn a_time_in_any_zone_graph_names_is_put_in_utc() {
        assert_eq!(
            utc("2026-10-20T03:00:00.0000000", "UTC").as_deref(),
            Some("2026-10-20T03:00:00Z")
        );
        assert_eq!(
            utc("2026-10-20T09:00:00.0000000", "E. Australia Standard Time").as_deref(),
            Some("2026-10-19T23:00:00Z")
        );
        assert_eq!(
            utc("2026-01-20T09:00:00", "Australia/Sydney").as_deref(),
            Some("2026-01-19T22:00:00Z")
        );
        assert_eq!(utc("not a time", "UTC"), None);
        assert_eq!(utc("2026-10-20T03:00:00", "tzone://Microsoft/Custom"), None);
    }

    /// The table is data from outside. Every zone it names must be one the
    /// built-in database has, or an event in it could not be placed.
    #[test]
    fn every_zone_in_the_table_is_one_the_database_knows() {
        assert!(table().len() > 100, "the table did not load");
        for (windows, iana) in table() {
            assert!(
                jiff::tz::TimeZone::get(iana).is_ok(),
                "`{windows}` maps to `{iana}`, which the zone database does not have"
            );
        }
    }
}
