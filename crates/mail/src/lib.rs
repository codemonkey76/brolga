//! What Brolga does with a message once it has one.
//!
//! So far that is making a message's HTML safe to show. Every message is
//! written by a stranger, and is treated as written by an attacker.

mod html;

pub use html::{
    ImageSource, RemoteContent, Sanitised, html_to_all_text, html_to_text, html_to_visible_text,
    image_sources, sanitise_html, text_to_html, text_to_paragraphs,
};
