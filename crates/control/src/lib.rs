//! The control channel.
//!
//! A running client listens on a local socket and answers one JSON object a
//! line, so scripts and other tools on the same machine can run its commands.
//! The protocol is written up in `docs/control-protocol.md`.

mod client;
mod endpoint;
mod protocol;
mod server;

pub use client::{Client, ClientError};
pub use endpoint::{Endpoint, default_endpoint};
pub use protocol::{ErrorBody, MAX_LINE_BYTES, Request, Response};
pub use server::{Server, ServerError};
