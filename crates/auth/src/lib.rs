//! Signing in to Microsoft 365.
//!
//! Brolga is a public client of the Microsoft identity platform: it holds no
//! secret of its own, and signs a person in through their own browser with
//! the authorization code flow and PKCE, or with a device code where no
//! browser can reach this machine. The documents each part follows are listed
//! in `SOURCES.md`.
//!
//! Refresh tokens are kept only in a [`SecretStore`]. Access tokens are kept
//! only in memory. Neither is ever written to a log, and the types that hold
//! them print as `[redacted]`.

mod accounts;
mod browser;
mod claims;
mod device;
mod endpoints;
mod error;
mod pkce;
mod secrets;
mod token;

pub use accounts::{AccountError, Accounts};
pub use browser::BrowserSignIn;
pub use claims::claims_challenge;
pub use device::DeviceSignIn;
pub use endpoints::{Authority, Registration, SIGN_IN_SCOPES};
pub use error::{AuthError, Problem};
pub use secrets::{MemorySecrets, SecretError, SecretStore};
pub use token::{Identity, TokenClient, Tokens};
