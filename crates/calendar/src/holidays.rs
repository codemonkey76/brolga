//! Public holidays, by country and by the regions of one.
//!
//! The days are not worked out here. They are carried as a table
//! (`data/holidays.json`) written by `tools/holidays/generate.py` from the
//! `holidays` library (SOURCES.md), for a span of years. A country's days
//! are those the whole of it keeps; a region has the days it adds to those,
//! and the days it does not keep.

use std::sync::LazyLock;

use serde::Deserialize;

#[derive(Deserialize)]
struct Table {
    source: String,
    first_year: i16,
    last_year: i16,
    countries: Vec<Country>,
}

#[derive(Deserialize)]
struct Country {
    code: String,
    name: String,
    days: Vec<(String, String)>,
    regions: Vec<Region>,
}

#[derive(Deserialize)]
struct Region {
    code: String,
    name: String,
    add: Vec<(String, String)>,
    drop: Vec<String>,
}

/// Read once. A table that cannot be read is no holidays at all, which a
/// test would not let past.
static TABLE: LazyLock<Table> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/holidays.json")).unwrap_or(Table {
        source: String::new(),
        first_year: 0,
        last_year: -1,
        countries: Vec::new(),
    })
});

/// A country whose holidays are known, and the regions of it that keep
/// days of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// ISO 3166-1, two letters: `AU`.
    pub code: &'static str,
    pub name: &'static str,
    /// Each region's code, as the library has it (`QLD`), and its name.
    pub regions: Vec<(&'static str, &'static str)>,
}

/// A day off, and what it is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holiday {
    /// `2026-08-12`.
    pub date: String,
    pub name: String,
}

/// Every country there are holidays for, by name.
pub fn places() -> Vec<Place> {
    let mut places: Vec<Place> = TABLE
        .countries
        .iter()
        .map(|country| Place {
            code: &country.code,
            name: &country.name,
            regions: country
                .regions
                .iter()
                .map(|region| (region.code.as_str(), region.name.as_str()))
                .collect(),
        })
        .collect();
    places.sort_by_key(|place| place.name);

    places
}

/// The first and the last year the table has.
pub fn years() -> (i16, i16) {
    (TABLE.first_year, TABLE.last_year)
}

/// Where the days came from, for whoever asks.
pub fn source() -> &'static str {
    &TABLE.source
}

fn found(
    country: &str,
    region: Option<&str>,
) -> Option<(&'static Country, Option<&'static Region>)> {
    let country = TABLE
        .countries
        .iter()
        .find(|each| each.code.eq_ignore_ascii_case(country))?;
    let region = match region {
        Some(code) => Some(
            country
                .regions
                .iter()
                .find(|each| each.code.eq_ignore_ascii_case(code))?,
        ),
        None => None,
    };

    Some((country, region))
}

/// What a place is called: the region where one is given, and otherwise
/// the country. `None` where there is no such place.
pub fn name_of(country: &str, region: Option<&str>) -> Option<&'static str> {
    let (country, region) = found(country, region)?;

    Some(region.map_or(country.name.as_str(), |region| region.name.as_str()))
}

/// The holidays of a place in one year, earliest first. `None` where there
/// is no such place; an empty list for a year the table does not reach.
pub fn of(country: &str, region: Option<&str>, year: i16) -> Option<Vec<Holiday>> {
    let (country, region) = found(country, region)?;
    let within = format!("{year:04}-");
    let mut days: Vec<Holiday> = country
        .days
        .iter()
        .filter(|(date, _)| date.starts_with(&within))
        .filter(|(date, _)| region.is_none_or(|region| !region.drop.contains(date)))
        .map(|(date, name)| Holiday {
            date: date.clone(),
            name: name.clone(),
        })
        .collect();
    for (date, name) in region.iter().flat_map(|region| &region.add) {
        if !date.starts_with(&within) {
            continue;
        }
        // A day the region calls something else is the region's to name.
        days.retain(|day| &day.date != date);
        days.push(Holiday {
            date: date.clone(),
            name: name.clone(),
        });
    }
    days.sort_by(|a, b| a.date.cmp(&b.date));

    Some(days)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(country: &str, region: Option<&str>, year: i16) -> Vec<(String, String)> {
        of(country, region, year)
            .unwrap()
            .into_iter()
            .map(|day| (day.date, day.name))
            .collect()
    }

    #[test]
    fn the_table_is_read_and_covers_the_years_ahead() {
        let (first, last) = years();
        assert!(first <= 2026 && last >= 2028, "{first} to {last}");
        assert!(places().len() > 200);
        assert!(source().contains("holidays"));
    }

    #[test]
    fn a_region_adds_its_own_days_to_the_countrys_and_leaves_out_what_it_does_not_keep() {
        let queensland = names("AU", Some("QLD"), 2026);
        for day in [
            ("2026-01-01", "New Year's Day"),
            ("2026-08-12", "The Royal Queensland Show"),
            ("2026-10-05", "King's Birthday"),
            ("2026-12-25", "Christmas Day"),
        ] {
            assert!(
                queensland.contains(&(day.0.to_owned(), day.1.to_owned())),
                "{day:?} in {queensland:?}"
            );
        }
        // The whole country keeps fewer days than any one state.
        let country = names("AU", None, 2026);
        assert!(country.len() < queensland.len());
        assert!(
            !country
                .iter()
                .any(|(_, name)| name == "The Royal Queensland Show")
        );
        // Anzac Day on a Sunday is kept on the Sunday by the country, and on the Monday in Queensland.
        assert!(
            names("AU", None, 2027)
                .iter()
                .any(|(date, _)| date == "2027-04-25")
        );
        assert!(
            !names("AU", Some("QLD"), 2027)
                .iter()
                .any(|(date, _)| date == "2027-04-25")
        );
        assert!(
            names("AU", Some("QLD"), 2027)
                .iter()
                .any(|(date, _)| date == "2027-04-26")
        );
    }

    #[test]
    fn days_are_in_order_one_to_a_date_and_within_the_year() {
        for place in places() {
            let regions =
                std::iter::once(None).chain(place.regions.iter().map(|(code, _)| Some(*code)));
            for region in regions {
                let days = of(place.code, region, 2027).unwrap();
                for pair in days.windows(2) {
                    assert!(
                        pair[0].date < pair[1].date,
                        "{} {region:?}: {pair:?}",
                        place.code
                    );
                }
                for day in &days {
                    assert!(
                        day.date.starts_with("2027-")
                            && day.date.len() == 10
                            && !day.name.is_empty()
                    );
                    assert!(
                        day.date.parse::<jiff::civil::Date>().is_ok(),
                        "{}",
                        day.date
                    );
                }
            }
        }
    }

    /// Looks like a short form rather than a name, by the rule
    /// tools/holidays/generate.py picks names by: capitals and no small
    /// letters (`B`, `RS`, `NÖ`), two letters at most (`St`), or one short
    /// word with two or more capitals (`FBiH`). Real short names such as
    /// Zug or Goa are not caught; nor is a name in a script with no
    /// capitals at all.
    fn abbreviated(name: &str) -> bool {
        let letters = name.chars().count();
        let capitals = name.chars().filter(|c| c.is_uppercase()).count();
        let cased = capitals > 0 || name.chars().any(char::is_lowercase);
        (capitals > 0 && !name.chars().any(char::is_lowercase))
            || (cased && letters <= 2)
            || (!name.contains(' ') && letters <= 5 && capitals >= 2)
    }

    #[test]
    fn every_region_is_offered_by_a_name_not_a_short_form() {
        let short: Vec<String> = places()
            .iter()
            .flat_map(|place| {
                place
                    .regions
                    .iter()
                    .filter(|(_, name)| abbreviated(name))
                    .map(|(code, name)| format!("{}/{code} {name}", place.code))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(short.is_empty(), "named by a short form: {short:?}");

        assert_eq!(name_of("NZ", Some("AUK")), Some("Auckland"));
        assert_eq!(name_of("AT", Some("1")), Some("Burgenland"));
        assert_eq!(name_of("MY", Some("01")), Some("Johor"));
        assert_eq!(name_of("BA", Some("SRP")), Some("Republika Srpska"));
        // Names that are short and right stay as they were.
        assert_eq!(name_of("CH", Some("ZG")), Some("Zug"));
        assert_eq!(name_of("FR", Some("974")), Some("La Réunion"));
        assert_eq!(name_of("IN", Some("TS")), Some("Telangāna"));
    }

    #[test]
    fn a_place_is_found_whatever_the_case_and_one_that_is_not_there_is_none() {
        assert_eq!(name_of("au", Some("qld")), Some("Queensland"));
        assert_eq!(name_of("AU", None), Some("Australia"));
        assert_eq!(name_of("AU", Some("ZZZ")), None);
        assert!(of("ZZ", None, 2026).is_none());
        assert_eq!(of("AU", None, 1990), Some(Vec::new()));
    }
}
