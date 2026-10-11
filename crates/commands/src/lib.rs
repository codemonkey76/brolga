//! The command registry.
//!
//! Everything the client can do is a command: a stable dotted id, a JSON
//! Schema for what it takes and what it gives back, and a description written
//! for an agent. The window, the keyboard, the CLI, the control channel and the
//! MCP server all run commands through one [`Registry`], so nothing can be
//! done by clicking that cannot also be done by an agent.

mod builtin;
mod registry;

pub use builtin::builtin_registry;
pub use registry::{
    Access, CommandError, CommandKind, CommandSpec, Handler, Registry, RegistryError,
    drafting_commands,
};
