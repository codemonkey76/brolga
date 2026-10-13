//! A client for Microsoft Graph v1.0.
//!
//! It knows how Graph wants to be talked to: a bearer token on every
//! request, immutable IDs, a few requests at a time, and patience when it is
//! told to slow down. It knows nothing about where the data goes; that is the
//! sync crate's business. Every endpoint used here is listed in `SOURCES.md`
//! with the document it was checked against.

mod calendar;
mod client;
mod delta;
mod error;
mod mail;
mod retry;

pub use calendar::{Attendee, Calendar, Event, Location, OnlineMeeting, ResponseStatus};
pub use client::{DRAFT_MARK, GraphClient, TokenSource};
pub use delta::{DeltaItem, DeltaPage, Next};
pub use error::GraphError;
pub use mail::{
    Attachment, DateTimeTimeZone, EmailAddress, FileAttachment, Flag, ItemBody, MailFolder,
    Message, Recipient, WELL_KNOWN_FOLDERS,
};
pub use retry::RetryPolicy;
