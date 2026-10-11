//! A file of vCards, read into contacts and written back.
#![no_main]

use brolga_contacts::{MOST, read, write};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|file: &str| {
    let first = read(file);
    assert!(first.contacts.len() <= MOST);

    // What was read can be written, and what was written can be read.
    read(&write(&first.contacts));
});
