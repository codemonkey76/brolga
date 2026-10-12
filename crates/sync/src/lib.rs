//! Keeping an account's local mail in step with the server.
//!
//! Sync asks Graph what has changed with delta queries and writes the answer
//! to the account's database a page at a time. Correctness comes before
//! freshness: a sync that is a minute late is fine, and one that loses or
//! duplicates a message is not. Three rules carry that:
//!
//! - A page and the place it leaves the sync are saved in one transaction,
//!   so a crash at any moment resumes without loss or repeat.
//! - Nothing is deleted on the server's say-so that the server did not name.
//!   When the server asks for a fresh start, everything is kept while the
//!   whole resource is read again, and only what that pass did not see goes.
//! - A message leaving one folder for another is never lost between them,
//!   whichever folder's changes arrive first.

mod calendar;
mod contacts;
mod mail;

pub use calendar::{CalendarSync, event_row};
pub use contacts::{ContactSync, contact_row};
pub use mail::{Doing, MailSync, Synced};

use brolga_graph::GraphError;
use brolga_store::StoreError;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("Microsoft 365 kept asking for a fresh sync of {0} and never let one finish.")]
    NeverSettled(String),
}

impl SyncError {
    /// True when trying again later, unchanged, may well work.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Graph(e) if e.is_transient())
    }
}
