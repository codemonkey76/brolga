//! The commands for the inbox's rules.
//!
//! Rules live on the server and run there, on mail as it arrives, whether
//! or not this client is open. Nothing of them is kept here: they are read
//! and changed while Microsoft 365 can be reached, and not otherwise.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use brolga_commands::{CommandError, CommandKind, CommandSpec, Handler, Registry, RegistryError};
use brolga_graph::GraphError;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::mailboxes::{Mailbox, Mailboxes};

/// The parts of a rule that hold people, which Graph wraps and these commands do not.
const PEOPLE: [&str; 5] = [
    "fromAddresses",
    "sentToAddresses",
    "forwardTo",
    "forwardAsAttachmentTo",
    "redirectTo",
];

/// The parts that hold a list of words to look for.
const WORDS: [(&str, &str); 7] = [
    ("senderContains", "the sender contains"),
    ("subjectContains", "the subject contains"),
    ("bodyContains", "the body contains"),
    ("bodyOrSubjectContains", "the subject or body contains"),
    ("headerContains", "a header contains"),
    ("recipientContains", "a recipient contains"),
    ("categories", "it is categorised"),
];

/// What a rule may ask that is true or false, and how each is said.
const FLAGS: [(&str, &str, &str); 17] = [
    (
        "hasAttachments",
        "it has attachments",
        "it has no attachments",
    ),
    ("sentToMe", "it is sent to me", "I am not on the To line"),
    (
        "sentOnlyToMe",
        "it is sent only to me",
        "it is not sent only to me",
    ),
    ("sentCcMe", "I am copied on it", "I am not copied on it"),
    (
        "sentToOrCcMe",
        "it is sent or copied to me",
        "it is neither sent nor copied to me",
    ),
    ("notSentToMe", "it is not sent to me", "it is sent to me"),
    (
        "isMeetingRequest",
        "it is an invitation",
        "it is not an invitation",
    ),
    (
        "isMeetingResponse",
        "it answers an invitation",
        "it does not answer an invitation",
    ),
    (
        "isAutomaticReply",
        "it is an automatic reply",
        "it is not an automatic reply",
    ),
    (
        "isAutomaticForward",
        "it was forwarded automatically",
        "it was not forwarded automatically",
    ),
    (
        "isNonDeliveryReport",
        "it reports a message not delivered",
        "it is not a delivery failure",
    ),
    (
        "isReadReceipt",
        "it is a read receipt",
        "it is not a read receipt",
    ),
    (
        "isApprovalRequest",
        "it asks for approval",
        "it does not ask for approval",
    ),
    ("isEncrypted", "it is encrypted", "it is not encrypted"),
    ("isSigned", "it is signed", "it is not signed"),
    (
        "isPermissionControlled",
        "it is rights-protected",
        "it is not rights-protected",
    ),
    ("isVoicemail", "it is a voicemail", "it is not a voicemail"),
];

fn failed(reason: impl std::fmt::Display) -> CommandError {
    CommandError::Failed(reason.to_string())
}

async fn open(
    mailboxes: &Mailboxes,
    account_id: Option<&str>,
) -> Result<Arc<Mailbox>, CommandError> {
    let account = mailboxes.account(account_id).map_err(failed)?;

    mailboxes.open(&account).await.map_err(failed)
}

fn not_reached(error: GraphError, then: &str) -> CommandError {
    if error.is_transient() {
        failed(format!(
            "Microsoft 365 could not be reached, so {then}. Rules are kept there and can only be read or changed while it can be."
        ))
    } else {
        failed(error)
    }
}

/// The folders' names by id, to say where a rule puts things.
fn folder_names(mailbox: &Mailbox) -> HashMap<String, String> {
    mailbox
        .db
        .folders()
        .map(|all| all.into_iter().map(|each| (each.id, each.name)).collect())
        .unwrap_or_default()
}

fn addresses(list: &Value) -> Vec<String> {
    list.as_array()
        .map(|all| {
            all.iter()
                .filter_map(|each| each["address"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn quoted(list: &Value) -> String {
    list.as_array()
        .map(|all| {
            all.iter()
                .filter_map(Value::as_str)
                .map(|each| format!("“{each}”"))
                .collect::<Vec<_>>()
                .join(" or ")
        })
        .unwrap_or_default()
}

/// What a rule asks of a message, in words. Nothing asked is every message.
fn conditions_said(part: &Value) -> Vec<String> {
    let mut said = Vec::new();
    let from = addresses(&part["fromAddresses"]);
    if !from.is_empty() {
        said.push(format!("it is from {}", from.join(" or ")));
    }
    let to = addresses(&part["sentToAddresses"]);
    if !to.is_empty() {
        said.push(format!("it is sent to {}", to.join(" or ")));
    }
    for (name, words) in WORDS {
        let list = quoted(&part[name]);
        if !list.is_empty() {
            said.push(format!("{words} {list}"));
        }
    }
    for (name, yes, no) in FLAGS {
        match part[name].as_bool() {
            Some(true) => said.push(yes.to_owned()),
            Some(false) => said.push(no.to_owned()),
            None => {}
        }
    }
    for (name, words) in [
        ("importance", "its importance is"),
        ("sensitivity", "its sensitivity is"),
        ("messageActionFlag", "it is flagged"),
    ] {
        if let Some(value) = part[name].as_str() {
            said.push(format!("{words} {value}"));
        }
    }
    if let Some(size) = part["withinSizeRange"].as_object() {
        said.push(format!(
            "it is between {} and {} KB",
            size.get("minimumSize")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            size.get("maximumSize")
                .and_then(Value::as_i64)
                .unwrap_or_default()
        ));
    }

    said
}

fn actions_said(part: &Value, folders: &HashMap<String, String>) -> Vec<String> {
    let mut said = Vec::new();
    for (name, words) in [
        ("moveToFolder", "move it to"),
        ("copyToFolder", "copy it to"),
    ] {
        if let Some(id) = part[name].as_str() {
            let folder = folders
                .get(id)
                .map_or("a folder this computer has not seen", String::as_str);
            said.push(format!("{words} {folder}"));
        }
    }
    let categories = quoted(&part["assignCategories"]);
    if !categories.is_empty() {
        said.push(format!(
            "categorise it {}",
            categories.replace(" or ", " and ")
        ));
    }
    if part["markAsRead"] == true {
        said.push("mark it read".to_owned());
    }
    if let Some(importance) = part["markImportance"].as_str() {
        said.push(format!("mark its importance {importance}"));
    }
    for (name, words) in [
        ("forwardTo", "forward it to"),
        ("forwardAsAttachmentTo", "forward it as an attachment to"),
        ("redirectTo", "redirect it to"),
    ] {
        let people = addresses(&part[name]);
        if !people.is_empty() {
            said.push(format!("{words} {}", people.join(" and ")));
        }
    }
    if part["delete"] == true {
        said.push("delete it".to_owned());
    }
    if part["permanentDelete"] == true {
        said.push("delete it for good".to_owned());
    }
    if part["stopProcessingRules"] == true {
        said.push("stop there".to_owned());
    }

    said
}

/// A part of a rule as these commands give it: people unwrapped to an
/// address and a name, and nothing that is null.
fn part_shown(part: &Value) -> Value {
    let mut shown = Map::new();
    for (name, value) in part.as_object().into_iter().flatten() {
        if value.is_null() || name.starts_with('@') {
            continue;
        }
        if PEOPLE.contains(&name.as_str()) {
            let people: Vec<Value> = value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|each| {
                    let address = each["emailAddress"]["address"].as_str()?;
                    Some(json!({ "address": address, "name": each["emailAddress"]["name"].as_str().unwrap_or_default() }))
                })
                .collect();
            shown.insert(name.clone(), Value::Array(people));
        } else {
            shown.insert(name.clone(), value.clone());
        }
    }

    Value::Object(shown)
}

/// A rule as these commands give it, with what it does said in a sentence.
fn shown(rule: &Value, folders: &HashMap<String, String>) -> Value {
    let conditions = part_shown(&rule["conditions"]);
    let exceptions = part_shown(&rule["exceptions"]);
    let actions = part_shown(&rule["actions"]);

    let when = conditions_said(&conditions);
    let unless = conditions_said(&exceptions);
    let then = actions_said(&actions, folders);
    let mut summary = if when.is_empty() {
        "For every message".to_owned()
    } else {
        format!("When {}", when.join(" and "))
    };
    if !unless.is_empty() {
        summary.push_str(&format!(", unless {}", unless.join(" or ")));
    }
    summary.push_str(": ");
    summary.push_str(&if then.is_empty() {
        "do nothing this client knows of".to_owned()
    } else {
        then.join(", ")
    });
    summary.push('.');

    json!({
        "id": rule["id"],
        "name": rule["displayName"].as_str().unwrap_or_default(),
        "enabled": rule["isEnabled"].as_bool().unwrap_or(true),
        "sequence": rule["sequence"].as_i64().unwrap_or_default(),
        "read_only": rule["isReadOnly"].as_bool().unwrap_or(false),
        "has_error": rule["hasError"].as_bool().unwrap_or(false),
        "conditions": conditions,
        "exceptions": exceptions,
        "actions": actions,
        "summary": summary
    })
}

/// A part of a rule as Graph takes it: people wrapped, and a folder named
/// by its well-known name turned into its id.
fn part_sent(mailbox: &Mailbox, part: Map<String, Value>) -> Result<Value, CommandError> {
    let mut sent = Map::new();
    for (name, value) in part {
        let value = if PEOPLE.contains(&name.as_str()) {
            let mut people = Vec::new();
            for each in value.as_array().into_iter().flatten() {
                let (address, called) = match each {
                    Value::String(address) => (address.as_str(), ""),
                    other => (
                        other["address"].as_str().unwrap_or_default(),
                        other["name"].as_str().unwrap_or_default(),
                    ),
                };
                let address = address.trim();
                if !address.contains('@') || address.contains(char::is_whitespace) {
                    return Err(failed(format!(
                        "`{address}` in {name} is not an email address."
                    )));
                }
                people.push(json!({ "emailAddress": { "address": address, "name": called } }));
            }
            Value::Array(people)
        } else if name == "moveToFolder" || name == "copyToFolder" {
            let folder = value.as_str().unwrap_or_default();
            let id = mailbox
                .db
                .folder_id(folder)
                .map_err(failed)?
                .ok_or_else(|| {
                    failed(format!(
                        "There is no folder `{folder}`. The folders are in mail.folders.list."
                    ))
                })?;
            Value::String(id)
        } else {
            value
        };
        sent.insert(name, value);
    }

    Ok(Value::Object(sent))
}

#[derive(Deserialize)]
struct Account {
    account_id: Option<String>,
}

async fn list(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let all = mailbox
        .graph
        .message_rules()
        .await
        .map_err(|e| not_reached(e, "the rules could not be read"))?;
    let folders = folder_names(&mailbox);
    let mut rules: Vec<Value> = all.iter().map(|rule| shown(rule, &folders)).collect();
    rules.sort_by_key(|rule| rule["sequence"].as_i64().unwrap_or_default());

    Ok(json!({ "rules": rules }))
}

#[derive(Deserialize)]
struct Create {
    account_id: Option<String>,
    name: String,
    enabled: Option<bool>,
    sequence: Option<i64>,
    conditions: Option<Map<String, Value>>,
    exceptions: Option<Map<String, Value>>,
    actions: Map<String, Value>,
}

async fn create(mailboxes: Arc<Mailboxes>, args: Create) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let name = args.name.trim().to_owned();
    if name.is_empty() {
        return Err(failed("A rule needs a name."));
    }
    // What is there now: a new rule runs after them unless told otherwise,
    // and if the answer to making it is lost, this says whether it was made.
    let before = mailbox
        .graph
        .message_rules()
        .await
        .map_err(|e| not_reached(e, "the rule was not made"))?;
    let last = before
        .iter()
        .filter_map(|rule| rule["sequence"].as_i64())
        .max()
        .unwrap_or_default();

    let mut rule = json!({
        "displayName": name,
        "sequence": args.sequence.unwrap_or(last + 1),
        "isEnabled": args.enabled.unwrap_or(true),
        "actions": part_sent(&mailbox, args.actions)?
    });
    if let Some(conditions) = args.conditions.filter(|part| !part.is_empty()) {
        rule["conditions"] = part_sent(&mailbox, conditions)?;
    }
    if let Some(exceptions) = args.exceptions.filter(|part| !part.is_empty()) {
        rule["exceptions"] = part_sent(&mailbox, exceptions)?;
    }

    let made = match mailbox.graph.create_rule(&rule).await {
        Ok(made) => made,
        Err(error) if error.is_transient() => {
            // Asked again blindly there would be two of it, so it is looked for.
            let now = mailbox.graph.message_rules().await.map_err(|_| {
                failed(
                    "Microsoft 365 stopped answering while the rule was being made, so it may or may not be there. \
                     Look at mail.rules.list before trying again.",
                )
            })?;
            now.into_iter()
                .find(|each| {
                    each["displayName"] == rule["displayName"]
                        && !before.iter().any(|old| old["id"] == each["id"])
                })
                .ok_or_else(|| {
                    failed("Microsoft 365 could not be reached, so the rule was not made. Try again when it can be.")
                })?
        }
        Err(error) => return Err(failed(error)),
    };

    Ok(json!({ "rule": shown(&made, &folder_names(&mailbox)) }))
}

#[derive(Deserialize)]
struct Update {
    account_id: Option<String>,
    rule_id: String,
    name: Option<String>,
    enabled: Option<bool>,
    sequence: Option<i64>,
    conditions: Option<Map<String, Value>>,
    exceptions: Option<Map<String, Value>>,
    actions: Option<Map<String, Value>>,
}

/// The rule of this id, which must be one that may be changed.
async fn changeable(mailbox: &Mailbox, rule_id: &str, then: &str) -> Result<Value, CommandError> {
    let all = mailbox
        .graph
        .message_rules()
        .await
        .map_err(|e| not_reached(e, then))?;
    let rule = all
        .into_iter()
        .find(|rule| rule["id"] == rule_id)
        .ok_or_else(|| {
            failed("There is no rule with that id. The rules are in mail.rules.list.")
        })?;
    if rule["isReadOnly"] == true {
        return Err(failed(
            "Microsoft 365 holds that rule as read-only: it can be seen here, and changed only where it was made.",
        ));
    }

    Ok(rule)
}

async fn update(mailboxes: Arc<Mailboxes>, args: Update) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    changeable(&mailbox, &args.rule_id, "the rule was not changed").await?;

    let mut changes = Map::new();
    if let Some(name) = args.name {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(failed("A rule needs a name."));
        }
        changes.insert("displayName".to_owned(), json!(name));
    }
    if let Some(enabled) = args.enabled {
        changes.insert("isEnabled".to_owned(), json!(enabled));
    }
    if let Some(sequence) = args.sequence {
        changes.insert("sequence".to_owned(), json!(sequence));
    }
    if let Some(conditions) = args.conditions {
        changes.insert("conditions".to_owned(), part_sent(&mailbox, conditions)?);
    }
    if let Some(exceptions) = args.exceptions {
        changes.insert("exceptions".to_owned(), part_sent(&mailbox, exceptions)?);
    }
    if let Some(actions) = args.actions {
        if actions.is_empty() {
            return Err(failed(
                "A rule has to do something: give at least one action.",
            ));
        }
        changes.insert("actions".to_owned(), part_sent(&mailbox, actions)?);
    }
    if changes.is_empty() {
        return Err(failed("Nothing to change was given."));
    }

    let changed = mailbox
        .graph
        .update_rule(&args.rule_id, &Value::Object(changes))
        .await
        .map_err(|e| not_reached(e, "the rule may not have been changed"))?;

    Ok(json!({ "rule": shown(&changed, &folder_names(&mailbox)) }))
}

#[derive(Deserialize)]
struct Delete {
    account_id: Option<String>,
    rule_id: String,
}

async fn delete(mailboxes: Arc<Mailboxes>, args: Delete) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    changeable(&mailbox, &args.rule_id, "the rule was not deleted").await?;
    mailbox
        .graph
        .delete_rule(&args.rule_id)
        .await
        .map_err(|e| not_reached(e, "the rule may not have been deleted"))?;

    Ok(json!({ "deleted": true }))
}

fn handler<A, F, Fut>(mailboxes: &Arc<Mailboxes>, run: F) -> Handler
where
    A: serde::de::DeserializeOwned + Send + 'static,
    F: Fn(Arc<Mailboxes>, A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value, CommandError>> + Send + 'static,
{
    let mailboxes = mailboxes.clone();

    Arc::new(move |args| {
        let running = serde_json::from_value::<A>(args).map(|args| run(mailboxes.clone(), args));

        Box::pin(async move {
            match running {
                Ok(running) => running.await,
                Err(error) => Err(failed(error)),
            }
        })
    })
}

fn words() -> Value {
    json!({ "type": "array", "items": { "type": "string", "minLength": 1, "maxLength": 255 }, "maxItems": 50 })
}

fn people() -> Value {
    json!({
        "type": "array",
        "maxItems": 50,
        "items": {
            "anyOf": [
                { "type": "string", "description": "An email address." },
                {
                    "type": "object",
                    "properties": { "address": { "type": "string" }, "name": { "type": "string" } },
                    "required": ["address"],
                    "additionalProperties": false
                }
            ]
        }
    })
}

/// What a rule may ask of a message: every name is Graph's own
/// ("messageRulePredicates resource type", SOURCES.md).
fn predicates(description: &str) -> Value {
    let mut properties = Map::new();
    for (name, _) in WORDS {
        properties.insert(name.to_owned(), words());
    }
    for (name, _, _) in FLAGS {
        properties.insert(name.to_owned(), json!({ "type": "boolean" }));
    }
    properties.insert("fromAddresses".to_owned(), people());
    properties.insert("sentToAddresses".to_owned(), people());
    properties.insert(
        "importance".to_owned(),
        json!({ "enum": ["low", "normal", "high"] }),
    );
    properties.insert(
        "sensitivity".to_owned(),
        json!({ "enum": ["normal", "personal", "private", "confidential"] }),
    );
    properties.insert(
        "messageActionFlag".to_owned(),
        json!({ "enum": ["any", "call", "doNotForward", "followUp", "fyi", "forward", "noResponseNecessary", "read", "reply", "replyToAll", "review"] }),
    );
    properties.insert(
        "withinSizeRange".to_owned(),
        json!({
            "type": "object",
            "description": "In kilobytes.",
            "properties": { "minimumSize": { "type": "integer", "minimum": 0 }, "maximumSize": { "type": "integer", "minimum": 0 } },
            "required": ["minimumSize", "maximumSize"],
            "additionalProperties": false
        }),
    );

    json!({ "type": "object", "description": description, "properties": properties, "additionalProperties": false })
}

/// What a rule may do ("messageRuleActions resource type", SOURCES.md).
fn actions() -> Value {
    json!({
        "type": "object",
        "description": "What is done to a message the rule applies to. At least one.",
        "properties": {
            "moveToFolder": { "type": "string", "description": "A folder's id from mail.folders.list, or a well-known name such as `archive`." },
            "copyToFolder": { "type": "string", "description": "As moveToFolder." },
            "assignCategories": words(),
            "markAsRead": { "type": "boolean" },
            "markImportance": { "enum": ["low", "normal", "high"] },
            "forwardTo": people(),
            "forwardAsAttachmentTo": people(),
            "redirectTo": people(),
            "delete": { "type": "boolean", "description": "Move to Deleted Items." },
            "permanentDelete": { "type": "boolean", "description": "Delete for good, with no way back." },
            "stopProcessingRules": { "type": "boolean", "description": "Rules later in the order are not tried." }
        },
        "minProperties": 1,
        "additionalProperties": false
    })
}

pub(crate) fn register(
    registry: &mut Registry,
    mailboxes: Arc<Mailboxes>,
) -> Result<(), RegistryError> {
    let account_id = json!({ "type": "string", "description": "Which account, from accounts.list. May be left out when there is only one." });
    let rule = json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "name": { "type": "string" },
            "enabled": { "type": "boolean" },
            "sequence": { "type": "integer", "description": "Rules run in this order, lowest first." },
            "read_only": { "type": "boolean", "description": "Microsoft 365 will not let this rule be changed or deleted here." },
            "has_error": { "type": "boolean", "description": "Microsoft 365 says the rule is broken, for instance its folder is gone." },
            "conditions": { "type": "object" },
            "exceptions": { "type": "object" },
            "actions": { "type": "object" },
            "summary": { "type": "string", "description": "What the rule does, as a sentence." }
        },
        "required": ["id", "name", "enabled", "sequence", "read_only", "conditions", "exceptions", "actions", "summary"]
    });
    let one = json!({ "type": "object", "properties": { "rule": rule }, "required": ["rule"] });
    const WHERE: &str = "Rules are kept by Microsoft 365 and run there on mail as it arrives, so this works only while it can be reached.";

    registry.register(
        CommandSpec {
            id: "mail.rules.create".to_owned(),
            title: "Create a rule".to_owned(),
            description: format!(
                "Make a rule for mail arriving in the inbox: what to look for (all of `conditions` must hold; leave them out for every message), \
                 what rules it out (`exceptions`), and what to do (`actions`). It runs after the rules already there unless `sequence` says otherwise. \
                 A rule that forwards, redirects or deletes acts on every matching message from then on without anyone looking. {WHERE}"
            ),
            kind: CommandKind::Action,
            // It goes on acting on mail, unseen, until it is turned off.
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id,
                    "name": { "type": "string", "minLength": 1, "maxLength": 255 },
                    "enabled": { "type": "boolean", "description": "On unless this is false." },
                    "sequence": { "type": "integer", "minimum": 1 },
                    "conditions": predicates("What must be true of a message, all of it. The words in one list are alternatives."),
                    "exceptions": predicates("A message any of these is true of is left alone."),
                    "actions": actions()
                },
                "required": ["name", "actions"],
                "additionalProperties": false
            }),
            result_schema: one.clone(),
            examples: vec![json!({
                "name": "Invoices to the archive",
                "conditions": { "subjectContains": ["invoice"], "fromAddresses": ["accounts@example.test"] },
                "actions": { "moveToFolder": "archive", "markAsRead": true }
            })],
        },
        handler(&mailboxes, create),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.rules.delete".to_owned(),
            title: "Delete a rule".to_owned(),
            description: format!("Delete one of the inbox's rules. Mail it has already dealt with stays as it is. To stop a rule for a while, turn it off with mail.rules.update instead. {WHERE}"),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id, "rule_id": { "type": "string", "description": "From mail.rules.list." } },
                "required": ["rule_id"],
                "additionalProperties": false
            }),
            result_schema: json!({ "type": "object", "properties": { "deleted": { "type": "boolean" } }, "required": ["deleted"] }),
            examples: vec![json!({ "rule_id": "rule-0002" })],
        },
        handler(&mailboxes, delete),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.rules.list".to_owned(),
            title: "List rules".to_owned(),
            description: format!(
                "The inbox's rules in the order they run, each with what it looks for, what rules it out and what it does, and the same said as a sentence. \
                 The parts use Microsoft Graph's own names (subjectContains, moveToFolder and so on), as mail.rules.create takes them. {WHERE}"
            ),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: Some("list_rules".to_owned()),
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id },
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "rules": { "type": "array", "items": rule } },
                "required": ["rules"]
            }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, list),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.rules.update".to_owned(),
            title: "Change a rule".to_owned(),
            description: format!(
                "Change one of the inbox's rules: turn it on or off, rename it, move it in the order, or replace what it looks for or does. \
                 Only what is given changes, but `conditions`, `exceptions` and `actions` are each replaced whole, so give all of one to change any of it; \
                 an empty `conditions` or `exceptions` clears it. A rule marked read_only cannot be changed. {WHERE}"
            ),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: Some("update_rule".to_owned()),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id,
                    "rule_id": { "type": "string", "description": "From mail.rules.list." },
                    "name": { "type": "string", "minLength": 1, "maxLength": 255 },
                    "enabled": { "type": "boolean" },
                    "sequence": { "type": "integer", "minimum": 1 },
                    "conditions": predicates("What must be true of a message, all of it."),
                    "exceptions": predicates("A message any of these is true of is left alone."),
                    "actions": actions()
                },
                "required": ["rule_id"],
                "additionalProperties": false
            }),
            result_schema: one,
            examples: vec![json!({ "rule_id": "rule-0001", "enabled": false })],
        },
        handler(&mailboxes, update),
    )?;

    Ok(())
}
