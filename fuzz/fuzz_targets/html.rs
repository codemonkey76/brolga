//! A message's HTML, made safe to show and turned to plain text.
#![no_main]

use brolga_mail::{RemoteContent, html_to_text, sanitise_html};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|html: &str| {
    let shown = sanitise_html(html, RemoteContent::Allowed);
    let blocked = sanitise_html(html, RemoteContent::Blocked);

    // What was made safe is still safe when read a second time: nothing
    // remote comes back out of a message it was taken out of.
    let again = sanitise_html(&blocked.html, RemoteContent::Blocked);
    assert_eq!(again.blocked_remote, 0, "{:?}", blocked.html);

    html_to_text(html);
    html_to_text(&shown.html);
});
