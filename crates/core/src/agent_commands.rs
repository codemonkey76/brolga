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

pub(crate) fn register(registry: &mut Registry, db: Arc<GlobalDb>) -> Result<(), RegistryError> {
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
