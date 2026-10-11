//! A client for Microsoft Graph v1.0.
//!
//! It knows how Graph wants to be talked to: a bearer token on every
//! request, immutable IDs, a few requests at a time, and patience when it is
//! told to slow down. It knows nothing about where the data goes; that is the
//! sync crate's business. Every endpoint used here is listed in `SOURCES.md`
//! with the document it was checked against.

mod client;
mod delta;
mod error;
mod mail;
mod retry;

pub use client::{GraphClient, TokenSource};
pub use delta::{DeltaItem, DeltaPage, Next};
pub use error::GraphError;
pub use mail::{
    Attachment, EmailAddress, FileAttachment, Flag, ItemBody, MailFolder, Message, Recipient,
    WELL_KNOWN_FOLDERS,
};
pub use retry::RetryPolicy;
