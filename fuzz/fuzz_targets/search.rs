//! What is typed in the search box, turned into a query for the store.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|typed: &str| {
    brolga_search::parse(typed);
});
