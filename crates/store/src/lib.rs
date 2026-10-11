//! What Brolga keeps on disk.
//!
//! The global database holds the list of accounts. Each account then has a
//! database of its own for its mail.
//!
//! No secret is ever written here. Refresh tokens live in the operating
//! system's secret store, and the tests check that none reaches this file.

mod account;
mod calendar;
mod drafts;
mod global;
mod migrate;
mod ops;

pub use account::{
    AccountDb, AttachmentRow, Body, ConversationRow, Folder, FolderChange, InlineImage,
    MessageChange, MessageRow, Order, Round, SyncState,
};
pub use calendar::{CalendarRow, EventRow};
pub use drafts::{Draft, DraftEdit, DraftFile};
pub use global::{Account, AgentLogEntry, GlobalDb};
pub use ops::{FailedOp, MessageEdit, Op};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Brolga's database could not be read or written: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Brolga's data folder could not be created: {0}")]
    Io(#[from] std::io::Error),
    #[error(
        "This database was written by a newer version of Brolga (schema {found}, this version knows {known}). \
         Update Brolga to open it."
    )]
    Newer { found: u32, known: u32 },
}
