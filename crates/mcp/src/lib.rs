//! The MCP server.
//!
//! An agent sees the client's commands as tools: a named tool for each common
//! job, and `list_commands` and `execute_command` for everything else. The
//! server either runs commands itself (headless) or passes them to a running
//! client over the control channel (attached).

mod backend;
mod policy;
mod server;
mod tools;

pub use backend::Backend;
pub use brolga_commands::Access;
pub use policy::Policy;
pub use server::{McpServer, ServeError, serve_stdio};
