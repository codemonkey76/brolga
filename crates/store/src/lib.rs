//! What Brolga keeps on disk.
//!
//! So far that is the global database: the list of accounts. Each account's
//! own database of mail, calendar and contacts arrives with sync.
//!
//! No secret is ever written here. Refresh tokens live in the operating
//! system's secret store, and the tests check that none reaches this file.

mod global;
mod migrate;

pub use global::{Account, GlobalDb};

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
