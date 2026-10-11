//! How a repeating event repeats, as the server writes it, and when it falls.
#![no_main]

use brolga_calendar::{Series, occurrences};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|json: &[u8]| {
    let Ok(event) = serde_json::from_slice::<serde_json::Value>(json) else {
        return;
    };
    let (Some(start), Some(end)) = (
        event["start"].as_str().and_then(|s| s.parse().ok()),
        event["end"].as_str().and_then(|s| s.parse().ok()),
    ) else {
        return;
    };
    let zone = event["zone"].as_str().unwrap_or("Australia/Brisbane");
    let Some(series) = Series::from_graph(start, end, zone, &event["recurrence"]) else {
        return;
    };

    let (Ok(after), Ok(before)) = (
        "2020-01-01T00:00:00Z".parse::<jiff::Timestamp>(),
        "2032-01-01T00:00:00Z".parse::<jiff::Timestamp>(),
    ) else {
        return;
    };
    if let Ok(found) = occurrences(&series, after, before, 500) {
        assert!(found.len() <= 500);
    }
});
