//! The record of what agents have asked this client to do.
//!
//! The MCP server writes an entry for every tool call an agent makes,
//! allowed or not, so that a person can see afterwards what was done in
//! their name and what was turned away.

use std::sync::Arc;

use brolga_commands::{CommandError, CommandKind, CommandSpec, Registry, RegistryError};
use brolga_store::{AgentLogEntry, GlobalDb};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
struct Record {
    client: String,
    command: String,
    args: Value,
    outcome: String,
    detail: Option<String>,
}

#[derive(Deserialize)]
struct List {
    limit: Option<u32>,
}

fn failed(reason: impl std::fmt::Display) -> CommandError {
    CommandError::Failed(reason.to_string())
}

/// The names of arguments whose values are a person's words or a file's
/// contents. Their length is recorded, never the words.
const PRIVATE: &[&str] = &[
    "body",
    "content_base64",
    "text",
    "internal_message",
    "external_message",
    "subject",
    "query",
    "typed",
    "description",
    "comment",
    "notes",
];
/// The longest other text kept whole.
const LONGEST: usize = 80;

/// Arguments as they are kept in the log: the shape of what was asked, with
/// anything long or private cut down to its size.
pub(crate) fn redacted(args: &Value) -> Value {
    fn cut(key: Option<&str>, value: &Value) -> Value {
        match value {
            Value::String(text) if key.is_some_and(|k| PRIVATE.contains(&k)) => {
                json!(format!("[{} characters]", text.chars().count()))
            }
            Value::String(text) if text.chars().count() > LONGEST => {
                let start: String = text.chars().take(LONGEST).collect();
                json!(format!("{start}… [{} characters]", text.chars().count()))
            }
            Value::Array(list) if list.len() > 20 => json!(format!("[{} items]", list.len())),
            Value::Array(list) => Value::Array(list.iter().map(|each| cut(key, each)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), cut(Some(k), v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    cut(None, args)
}

/// How long a person is given to answer before the answer is no.
const ANSWER_WITHIN: std::time::Duration = std::time::Duration::from_secs(120);
/// A window that has not looked for questions for this long is not there
/// to be asked.
const WINDOW_GONE_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

/// Something an agent wants to do that needs a person's say-so first.
struct Question {
    id: u64,
    client: String,
    command: String,
    /// As asked, with anything long or private cut down to its size.
    args: Value,
    answer: tokio::sync::oneshot::Sender<bool>,
}

/// The questions waiting for a person, and when a window last looked.
#[derive(Default)]
struct Questions {
    next: u64,
    waiting: Vec<Question>,
    window_looked: Option<std::time::Instant>,
}

#[derive(Deserialize)]
struct Ask {
    client: String,
    command: String,
    args: Value,
}

#[derive(Deserialize)]
struct Answer {
    id: u64,
    allow: bool,
}

fn lock(
    questions: &std::sync::Mutex<Questions>,
) -> Result<std::sync::MutexGuard<'_, Questions>, CommandError> {
    questions
        .lock()
        .map_err(|_| failed("the list of questions is broken"))
}

fn register_questions(registry: &mut Registry) -> Result<(), RegistryError> {
    let questions = Arc::new(std::sync::Mutex::new(Questions::default()));
    let asking = questions.clone();
    registry.register(
        CommandSpec {
            id: "app.confirmations.ask".to_owned(),
            title: "Ask the user's say-so".to_owned(),
            description: "Ask the person at the window whether an agent may run a command that reaches other people or cannot be undone, and wait for the answer. \
                          Used by the MCP server; an agent cannot run it. The answer is no if nobody answers in two minutes, or if no window is open to ask in."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "client": { "type": "string", "maxLength": 200 },
                    "command": { "type": "string", "maxLength": 200 },
                    "args": {}
                },
                "required": ["client", "command", "args"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "allowed": { "type": "boolean" }, "reason": { "type": "string" } },
                "required": ["allowed", "reason"]
            }),
            // With no window looking, it is answered at once: no.
            examples: vec![json!({ "client": "example-agent", "command": "mail.drafts.send", "args": { "draft_id": "draft-1" } })],
        },
        Arc::new(move |args| {
            let questions = asking.clone();
            Box::pin(async move {
                let ask: Ask = serde_json::from_value(args).map_err(failed)?;
                let (answer, answered) = tokio::sync::oneshot::channel();
                let id = {
                    let mut questions = lock(&questions)?;
                    let watched = questions.window_looked.is_some_and(|at| at.elapsed() < WINDOW_GONE_AFTER);
                    if !watched {
                        return Ok(json!({ "allowed": false, "reason": "There is no window open to ask the user in." }));
                    }
                    questions.next += 1;
                    let id = questions.next;
                    questions.waiting.push(Question {
                        id,
                        client: ask.client,
                        command: ask.command,
                        args: redacted(&ask.args),
                        answer,
                    });
                    id
                };

                let allowed = tokio::time::timeout(ANSWER_WITHIN, answered).await;
                // Answered or not, it is no longer waiting.
                if let Ok(mut questions) = questions.lock() {
                    questions.waiting.retain(|q| q.id != id);
                }

                Ok(match allowed {
                    Ok(Ok(true)) => json!({ "allowed": true, "reason": "The user allowed it." }),
                    Ok(_) => json!({ "allowed": false, "reason": "The user did not allow it." }),
                    Err(_) => json!({ "allowed": false, "reason": "The user did not answer in time." }),
                })
            })
        }),
    )?;

    let listing = questions.clone();
    registry.register(
        CommandSpec {
            id: "app.confirmations.list".to_owned(),
            title: "Questions waiting".to_owned(),
            description: "What agents are waiting for the user's say-so on: for each, the program asking, the command and its arguments with anything private cut out. \
                          The window asks this every few seconds, which is also how the client knows there is a window to ask in."
                .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "integer" },
                                "client": { "type": "string" },
                                "command": { "type": "string" },
                                "args": {}
                            },
                            "required": ["id", "client", "command", "args"]
                        }
                    }
                },
                "required": ["questions"]
            }),
            examples: vec![json!({})],
        },
        Arc::new(move |_| {
            let questions = listing.clone();
            Box::pin(async move {
                let mut questions = lock(&questions)?;
                questions.window_looked = Some(std::time::Instant::now());
                let waiting: Vec<Value> = questions
                    .waiting
                    .iter()
                    .map(|q| json!({ "id": q.id, "client": q.client, "command": q.command, "args": q.args }))
                    .collect();

                Ok(json!({ "questions": waiting }))
            })
        }),
    )?;

    registry.register(
        CommandSpec {
            id: "app.confirmations.answer".to_owned(),
            title: "Answer a question".to_owned(),
            description: "Say whether an agent may go ahead with what it asked. The person at the window does this; an agent cannot. \
                          `answered` is false if the question is no longer waiting."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "integer" }, "allow": { "type": "boolean" } },
                "required": ["id", "allow"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "answered": { "type": "boolean" } },
                "required": ["answered"]
            }),
            examples: vec![json!({ "id": 0, "allow": false })],
        },
        Arc::new(move |args| {
            let questions = questions.clone();
            Box::pin(async move {
                let answer: Answer = serde_json::from_value(args).map_err(failed)?;
                let mut questions = lock(&questions)?;
                let Some(at) = questions.waiting.iter().position(|q| q.id == answer.id) else {
                    return Ok(json!({ "answered": false }));
                };
                let question = questions.waiting.remove(at);
                let _ = question.answer.send(answer.allow);

                Ok(json!({ "answered": true }))
            })
        }),
    )?;

    Ok(())
}

pub(crate) fn register(registry: &mut Registry, db: Arc<GlobalDb>) -> Result<(), RegistryError> {
    register_questions(registry)?;

    let record_db = db.clone();
    registry.register(
        CommandSpec {
            id: "app.agent_log.record".to_owned(),
            title: "Record an agent's command".to_owned(),
            description: "Write down a command an agent asked for and what became of it. Used by the MCP server; an agent cannot run it. \
                          The arguments are cut down before they are kept: a person's words and a file's contents are recorded only as a length."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "client": { "type": "string", "maxLength": 200 },
                    "command": { "type": "string", "maxLength": 200 },
                    "args": {},
                    "outcome": { "enum": ["ok", "refused", "failed"] },
                    "detail": { "type": "string" }
                },
                "required": ["client", "command", "args", "outcome"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "recorded": { "type": "boolean" } },
                "required": ["recorded"]
            }),
            examples: vec![json!({
                "client": "example-agent",
                "command": "mail.drafts.edit",
                "args": { "draft_id": "draft-1", "body": "Some private words." },
                "outcome": "refused",
                "detail": "This agent may only read."
            })],
        },
        Arc::new(move |args| {
            let db = record_db.clone();
            Box::pin(async move {
                let record: Record = serde_json::from_value(args).map_err(failed)?;
                let detail: String = record.detail.unwrap_or_default().chars().take(500).collect();
                db.log_agent_command(&AgentLogEntry {
                    at: String::new(),
                    client: record.client,
                    command: record.command,
                    args: redacted(&record.args).to_string(),
                    outcome: record.outcome,
                    detail,
                })
                .map_err(failed)?;

                Ok(json!({ "recorded": true }))
            })
        }),
    )?;

    registry.register(
        CommandSpec {
            id: "app.agent_log.list".to_owned(),
            title: "Agent activity".to_owned(),
            description: "What agents have asked this client to do, the latest first: when, which program, which command, its arguments with \
                          anything private cut out, and whether it ran (`ok`), was not allowed (`refused`) or did not work (`failed`)."
                .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Default 100." } },
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "entries": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "at": { "type": "string", "description": "UTC." },
                                "client": { "type": "string" },
                                "command": { "type": "string" },
                                "args": { "description": "As asked, with anything long or private cut down to its size." },
                                "outcome": { "enum": ["ok", "refused", "failed"] },
                                "detail": { "type": "string" }
                            },
                            "required": ["at", "client", "command", "args", "outcome", "detail"]
                        }
                    }
                },
                "required": ["entries"]
            }),
            examples: vec![json!({})],
        },
        Arc::new(move |args| {
            let db = db.clone();
            Box::pin(async move {
                let list: List = serde_json::from_value(args).map_err(failed)?;
                let entries: Vec<Value> = db
                    .agent_log(list.limit.unwrap_or(100).min(500))
                    .map_err(failed)?
                    .into_iter()
                    .map(|entry| {
                        json!({
                            "at": entry.at,
                            "client": entry.client,
                            "command": entry.command,
                            "args": serde_json::from_str::<Value>(&entry.args).unwrap_or(Value::Null),
                            "outcome": entry.outcome,
                            "detail": entry.detail,
                        })
                    })
                    .collect();

                Ok(json!({ "entries": entries }))
            })
        }),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_private_or_long_is_kept_only_as_its_size() {
        let args = json!({
            "draft_id": "draft-1",
            "body": "Dear Sam, the offer is $40,000.",
            "to": [{ "name": "Sam", "address": "sam@example.test" }],
            "content_base64": "QUJD".repeat(1000),
            "path": "/".to_owned() + &"x".repeat(200),
            "message_ids": (0..50).map(|n| n.to_string()).collect::<Vec<_>>(),
            "is_read": true
        });

        let kept = redacted(&args);
        let text = kept.to_string();

        assert_eq!(kept["draft_id"], "draft-1");
        assert_eq!(kept["body"], "[31 characters]");
        assert_eq!(kept["content_base64"], "[4000 characters]");
        assert_eq!(kept["to"][0]["address"], "sam@example.test");
        assert_eq!(kept["message_ids"], "[50 items]");
        assert_eq!(kept["is_read"], true);
        assert!(kept["path"].as_str().unwrap().ends_with("[201 characters]"));
        assert!(
            !text.contains("40,000") && !text.contains("QUJDQUJD"),
            "{text}"
        );
    }
}
