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
//!
//! One folder or calendar that cannot be synced does not stop the others.

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

    /// True when the trouble is not with the one folder or calendar being
    /// asked after but with the whole account, or with this computer: no
    /// connection, no sign-in, a mailbox told to slow down, or a database
    /// that cannot be written. Asking after the next folder would only meet
    /// it again.
    pub fn is_account_wide(&self) -> bool {
        match self {
            Self::Store(_) => true,
            Self::Graph(error) => matches!(
                error,
                GraphError::Network(_)
                    | GraphError::Token(_)
                    | GraphError::Unauthorized { .. }
                    | GraphError::Busy { status: 429 }
            ),
            Self::NeverSettled(_) => false,
        }
    }

    /// True when the server failed, which may be its trouble with one
    /// folder or the start of an outage: only more of the same says which.
    fn is_server_failure(&self) -> bool {
        matches!(self, Self::Graph(GraphError::Busy { .. })) && !self.is_account_wide()
    }
}

/// How many folders or calendars in a row the server may fail on before
/// the rest are left for the next sync. One that fails every time is its
/// own trouble; several running is an outage, and not one to lean on.
const MOST_SERVER_FAILURES_IN_A_ROW: u32 = 3;

/// What a sync of every folder, or every calendar, did, and what stood in
/// its way. One that cannot be synced does not stop the ones after it.
#[derive(Debug, Default)]
pub struct Outcome {
    pub synced: Synced,
    /// What could not be synced, in the order it was tried: each by the
    /// name a person knows it by, and why.
    pub failed: Vec<(String, SyncError)>,
    /// True if the sync stopped before everything had been tried, because
    /// the trouble was the whole account's and not one folder's.
    pub stopped: bool,
    server_failures_in_a_row: u32,
}

impl Outcome {
    /// As one answer: what was done if all of it was, and otherwise the
    /// first thing that went wrong.
    pub fn into_result(self) -> Result<Synced, SyncError> {
        match self.failed.into_iter().next() {
            Some((_, error)) => Err(error),
            None => Ok(self.synced),
        }
    }

    /// What went wrong in words for a person: the first of it, and how many
    /// more there were. Nothing if nothing did.
    pub fn problem(&self) -> Option<String> {
        let (what, error) = self.failed.first()?;
        // The whole account's trouble is not any one folder's to be named for.
        if error.is_account_wide() {
            return Some(error.to_string());
        }

        Some(match self.failed.len() - 1 {
            0 => format!("{what} could not be synced. {error}"),
            1 => format!("{what} and one other could not be synced. {error}"),
            more => format!("{what} and {more} others could not be synced. {error}"),
        })
    }

    pub(crate) fn done(&mut self, synced: Synced) {
        self.synced.add(synced);
        self.server_failures_in_a_row = 0;
    }

    /// Note what could not be synced. True if nothing more should be tried.
    pub(crate) fn fail(&mut self, what: &str, error: SyncError) -> bool {
        if error.is_server_failure() {
            self.server_failures_in_a_row += 1;
        } else {
            self.server_failures_in_a_row = 0;
        }
        self.stopped = error.is_account_wide()
            || self.server_failures_in_a_row >= MOST_SERVER_FAILURES_IN_A_ROW;
        self.failed.push((what.to_owned(), error));

        self.stopped
    }
}
