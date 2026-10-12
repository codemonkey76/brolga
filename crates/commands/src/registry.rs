use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use jsonschema::Validator;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What sort of thing a command is, which decides how carefully it is let run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// Reads and changes nothing.
    Query,
    /// Changes the user's data.
    Action,
    /// Moves the window, so somebody watching can see what an agent is doing.
    Navigation,
}

/// How far an agent must be trusted before it may run a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// Reads, and changes nothing but this computer's own copy.
    Read,
    /// Writes drafts, which reach nobody until a person or a fully trusted
    /// agent sends them.
    Draft,
    /// Everything else: changes to the mailbox, sending, settings.
    Full,
}

/// The commands that only write drafts. Kept in one place, by id, so that a
/// new command is fully trusted work until someone decides otherwise.
const DRAFTING: &[&str] = &[
    "mail.drafts.create",
    "mail.drafts.edit",
    "mail.drafts.discard",
    "mail.drafts.open",
    "mail.drafts.files.add",
    "mail.drafts.files.remove",
];

/// A command as it is described to whoever might run it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSpec {
    /// Stable and dotted: `mail.reply_all`. Never reused for something else.
    pub id: String,
    pub title: String,
    /// Written for an agent choosing between commands, not for a menu.
    pub description: String,
    pub kind: CommandKind,
    /// Sending, deleting and the like, which wait for a person's say-so.
    pub consequential: bool,
    /// The named MCP tool that runs this, where it has one. Every command is
    /// reachable through `execute_command` whether it has or not.
    pub mcp_tool: Option<String>,
    pub args_schema: Value,
    pub result_schema: Value,
    /// Arguments the command accepts as they stand. The registry's own test
    /// runs each one, so a command with none fails the build.
    pub examples: Vec<Value>,
}

impl CommandSpec {
    /// How far an agent must be trusted to run this.
    pub fn access(&self) -> Access {
        if self.kind == CommandKind::Query {
            Access::Read
        } else if DRAFTING.contains(&self.id.as_str()) {
            Access::Draft
        } else {
            Access::Full
        }
    }
}

/// The ids `Access::Draft` is given to, for a test to hold against the
/// commands there really are.
pub fn drafting_commands() -> &'static [&'static str] {
    DRAFTING
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CommandError {
    #[error("there is no command called `{0}`")]
    UnknownCommand(String),
    #[error("the arguments do not fit `{id}`: {reason}")]
    InvalidArguments { id: String, reason: String },
    #[error("{0}")]
    Failed(String),
    #[error(
        "`{0}` is the person's own to do, in Brolga's window. It cannot be run from the command line, the control channel or an agent."
    )]
    WindowOnly(String),
    #[error("`{id}` gave back something its own schema does not allow: {reason}")]
    InvalidResult { id: String, reason: String },
}

impl CommandError {
    /// A short fixed name for the wire, so a caller need not read the message.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownCommand(_) => "unknown_command",
            Self::InvalidArguments { .. } => "invalid_arguments",
            Self::Failed(_) => "failed",
            Self::WindowOnly(_) => "window_only",
            Self::InvalidResult { .. } => "invalid_result",
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("`{0}` is not a command id: lower-case words joined by dots, such as `mail.reply_all`")]
    BadId(String),
    #[error("`{0}` is registered twice")]
    Duplicate(String),
    #[error("the MCP tool name `{tool}` is used by both `{first}` and `{second}`")]
    DuplicateTool {
        tool: String,
        first: String,
        second: String,
    },
    #[error("the {which} schema of `{id}` is not a JSON Schema: {reason}")]
    BadSchema {
        id: String,
        which: &'static str,
        reason: String,
    },
}

pub type Handler =
    Arc<dyn Fn(Value) -> BoxFuture<'static, Result<Value, CommandError>> + Send + Sync>;

struct Entry {
    spec: CommandSpec,
    args: Validator,
    result: Validator,
    handler: Handler,
}

#[derive(Default)]
pub struct Registry {
    entries: BTreeMap<String, Entry>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, spec: CommandSpec, handler: Handler) -> Result<(), RegistryError> {
        if !is_command_id(&spec.id) {
            return Err(RegistryError::BadId(spec.id));
        }
        if self.entries.contains_key(&spec.id) {
            return Err(RegistryError::Duplicate(spec.id));
        }
        if let Some(tool) = &spec.mcp_tool
            && let Some(other) = self
                .entries
                .values()
                .find(|e| e.spec.mcp_tool.as_ref() == Some(tool))
        {
            return Err(RegistryError::DuplicateTool {
                tool: tool.clone(),
                first: other.spec.id.clone(),
                second: spec.id,
            });
        }

        let compile = |schema: &Value, which: &'static str| {
            jsonschema::validator_for(schema).map_err(|e| RegistryError::BadSchema {
                id: spec.id.clone(),
                which,
                reason: e.to_string(),
            })
        };
        let args = compile(&spec.args_schema, "arguments")?;
        let result = compile(&spec.result_schema, "result")?;

        self.entries.insert(
            spec.id.clone(),
            Entry {
                spec,
                args,
                result,
                handler,
            },
        );
        Ok(())
    }

    /// Every command, in id order.
    pub fn specs(&self) -> impl Iterator<Item = &CommandSpec> {
        self.entries.values().map(|e| &e.spec)
    }

    pub fn spec(&self, id: &str) -> Option<&CommandSpec> {
        self.entries.get(id).map(|e| &e.spec)
    }

    /// The command a named MCP tool runs.
    pub fn spec_for_tool(&self, tool: &str) -> Option<&CommandSpec> {
        self.specs().find(|s| s.mcp_tool.as_deref() == Some(tool))
    }

    /// Run a command for a caller that is not the window: the command line,
    /// the control channel, the MCP server. The arguments are checked against
    /// its schema before it runs and what it gives back is checked after, so
    /// neither side of the contract can drift without something saying so.
    ///
    /// What is the person's own side of a say-so
    /// ([`WINDOW_ONLY`](crate::agents::WINDOW_ONLY)) is refused here, so that
    /// nothing reaching the client from outside can answer for them.
    pub async fn execute(&self, id: &str, args: Value) -> Result<Value, CommandError> {
        if crate::agents::WINDOW_ONLY.contains(&id) {
            return Err(CommandError::WindowOnly(id.to_owned()));
        }

        self.run(id, args).await
    }

    /// Run a command for the window, which is where the person is. Only
    /// the window's own shell calls this; everything else calls
    /// [`execute`](Self::execute).
    pub async fn execute_from_window(&self, id: &str, args: Value) -> Result<Value, CommandError> {
        self.run(id, args).await
    }

    async fn run(&self, id: &str, args: Value) -> Result<Value, CommandError> {
        let entry = self
            .entries
            .get(id)
            .ok_or_else(|| CommandError::UnknownCommand(id.to_owned()))?;

        if let Err(e) = entry.args.validate(&args) {
            return Err(CommandError::InvalidArguments {
                id: id.to_owned(),
                reason: e.to_string(),
            });
        }

        let result = (entry.handler)(args).await?;

        if let Err(e) = entry.result.validate(&result) {
            return Err(CommandError::InvalidResult {
                id: id.to_owned(),
                reason: e.to_string(),
            });
        }

        Ok(result)
    }
}

fn is_command_id(id: &str) -> bool {
    let word = |w: &str| {
        w.starts_with(|c: char| c.is_ascii_lowercase())
            && w.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };

    id.contains('.') && id.split('.').all(word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(id: &str) -> CommandSpec {
        CommandSpec {
            id: id.to_owned(),
            title: "Echo".to_owned(),
            description: "Gives back the text it was given.".to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "additionalProperties": false
            }),
            result_schema: json!({ "type": "string" }),
            examples: vec![json!({ "text": "hello" })],
        }
    }

    fn echo() -> Handler {
        Arc::new(|args| Box::pin(async move { Ok(args["text"].clone()) }))
    }

    #[tokio::test]
    async fn a_command_runs_with_arguments_that_fit() {
        let mut registry = Registry::new();
        registry.register(spec("test.echo"), echo()).unwrap();

        assert_eq!(
            registry
                .execute("test.echo", json!({ "text": "hello" }))
                .await,
            Ok(json!("hello"))
        );
    }

    #[tokio::test]
    async fn arguments_that_do_not_fit_never_reach_the_command() {
        let mut registry = Registry::new();
        let reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = reached.clone();
        registry
            .register(
                spec("test.echo"),
                Arc::new(move |_| {
                    flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    Box::pin(async { Ok(json!("")) })
                }),
            )
            .unwrap();

        let error = registry
            .execute("test.echo", json!({ "text": 5 }))
            .await
            .unwrap_err();

        assert_eq!(error.code(), "invalid_arguments");
        assert!(!reached.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_result_outside_the_schema_is_an_error() {
        let mut registry = Registry::new();
        registry
            .register(
                spec("test.echo"),
                Arc::new(|_| Box::pin(async { Ok(json!(5)) })),
            )
            .unwrap();

        let error = registry
            .execute("test.echo", json!({ "text": "hello" }))
            .await
            .unwrap_err();

        assert_eq!(error.code(), "invalid_result");
    }

    #[tokio::test]
    async fn an_unknown_command_is_named_in_the_error() {
        let error = Registry::new()
            .execute("mail.nothing", json!({}))
            .await
            .unwrap_err();

        assert_eq!(
            error,
            CommandError::UnknownCommand("mail.nothing".to_owned())
        );
    }

    #[tokio::test]
    async fn what_is_the_persons_own_runs_only_from_the_window() {
        let mut registry = Registry::new();
        let mut own = spec("app.confirmations.answer");
        own.kind = CommandKind::Action;
        registry.register(own, echo()).unwrap();
        let args = json!({ "text": "yes" });

        let outside = registry
            .execute("app.confirmations.answer", args.clone())
            .await
            .unwrap_err();

        assert_eq!(outside.code(), "window_only");
        assert!(outside.to_string().contains("window"), "{outside}");
        assert_eq!(
            registry
                .execute_from_window("app.confirmations.answer", args)
                .await,
            Ok(json!("yes"))
        );
    }

    #[test]
    fn an_id_is_lower_case_words_joined_by_dots() {
        for good in ["mail.reply_all", "calendar.event.create", "app.v2.ping"] {
            assert!(is_command_id(good), "{good}");
        }
        for bad in [
            "mail",
            "Mail.reply",
            "mail..reply",
            "mail.reply-all",
            ".mail",
            "mail.",
            "mail.2go",
        ] {
            assert!(!is_command_id(bad), "{bad}");
        }
    }

    #[test]
    fn an_id_cannot_be_registered_twice() {
        let mut registry = Registry::new();
        registry.register(spec("test.echo"), echo()).unwrap();

        assert_eq!(
            registry.register(spec("test.echo"), echo()),
            Err(RegistryError::Duplicate("test.echo".to_owned()))
        );
    }

    #[test]
    fn two_commands_cannot_share_a_tool_name() {
        let mut registry = Registry::new();
        let mut first = spec("test.echo");
        first.mcp_tool = Some("echo".to_owned());
        let mut second = spec("test.echo_again");
        second.mcp_tool = Some("echo".to_owned());
        registry.register(first, echo()).unwrap();

        assert!(matches!(
            registry.register(second, echo()),
            Err(RegistryError::DuplicateTool { .. })
        ));
    }

    #[test]
    fn a_schema_that_is_not_a_schema_is_refused() {
        let mut bad = spec("test.echo");
        bad.args_schema = json!({ "type": "no-such-type" });

        assert!(matches!(
            Registry::new().register(bad, echo()),
            Err(RegistryError::BadSchema {
                which: "arguments",
                ..
            })
        ));
    }
}
