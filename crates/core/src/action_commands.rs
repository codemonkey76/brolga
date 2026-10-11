//! The commands that change mail.
//!
//! Each makes its change on this computer at once, queues it for the server
//! and returns; the outbox sends it. A person sees the change straight away
//! and never waits on the network for it.

use std::future::Future;
use std::sync::Arc;

use brolga_commands::{CommandError, CommandKind, CommandSpec, Handler, Registry, RegistryError};
use brolga_store::MessageEdit;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mailboxes::{Mailbox, Mailboxes};

/// The most messages one action may change.
const MOST: usize = 500;

#[derive(Deserialize)]
struct SetRead {
    account_id: Option<String>,
    message_ids: Vec<String>,
    is_read: bool,
}

#[derive(Deserialize)]
struct SetFlag {
    account_id: Option<String>,
    message_ids: Vec<String>,
    flag: String,
}

#[derive(Deserialize)]
struct Move {
    account_id: Option<String>,
    message_ids: Vec<String>,
    folder_id: String,
}

#[derive(Deserialize)]
struct SetCategories {
    account_id: Option<String>,
    message_ids: Vec<String>,
    categories: Vec<String>,
}

#[derive(Deserialize)]
struct Purge {
    account_id: Option<String>,
    message_ids: Vec<String>,
}

#[derive(Deserialize)]
struct Undo {
    account_id: Option<String>,
    undo_id: String,
}

#[derive(Deserialize)]
struct Account {
    account_id: Option<String>,
}

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

/// Send what is queued, without making the caller wait for it.
fn send_soon(mailbox: &Arc<Mailbox>) {
    let mailbox = mailbox.clone();
    tokio::spawn(async move {
        let _ = mailbox.outbox.drain().await;
    });
}

/// Make an edit to some messages, queue it, and say how to undo it.
async fn edit(
    mailboxes: Arc<Mailboxes>,
    account_id: Option<String>,
    message_ids: Vec<String>,
    edit: MessageEdit,
) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, account_id.as_deref()).await?;
    let action = mailbox.db.new_action().map_err(failed)?;
    let changed = mailbox
        .db
        .queue_edit(&action, &message_ids, &edit)
        .map_err(failed)?;
    if changed > 0 {
        send_soon(&mailbox);
    }

    Ok(json!({ "changed": changed, "undo_id": action }))
}

async fn move_messages(mailboxes: Arc<Mailboxes>, args: Move) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let folder = mailbox
        .db
        .folder_id(&args.folder_id)
        .map_err(failed)?
        .ok_or_else(|| {
            failed(format!(
                "There is no folder `{}` in this account.",
                args.folder_id
            ))
        })?;

    edit(
        mailboxes,
        args.account_id,
        args.message_ids,
        MessageEdit::Move(folder),
    )
    .await
}

/// The mailbox's categories, as `[{name, color}]`: from Microsoft 365 when
/// it can be reached, and otherwise as they were when last it could.
async fn categories(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    if let Ok(list) = mailbox.graph.master_categories().await {
        let list: Vec<Value> = list
            .into_iter()
            .map(|(name, color)| json!({ "name": name, "color": color }))
            .collect();
        let _ = mailbox
            .db
            .set_setting("categories", &Value::Array(list).to_string());
    }
    let kept = mailbox.db.setting("categories").map_err(failed)?;
    let list: Value = kept
        .and_then(|kept| serde_json::from_str(&kept).ok())
        .unwrap_or_else(|| json!([]));

    Ok(json!({ "categories": list }))
}

/// Delete for good. This is not queued: it cannot be undone, so it is done
/// while the person waits and they are told at once whether it happened.
async fn purge(mailboxes: Arc<Mailboxes>, args: Purge) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let mut deleted = 0;
    for id in &args.message_ids {
        mailbox.graph.delete_permanently(id).await.map_err(|e| {
            failed(format!(
                "{deleted} of {} were deleted before this went wrong, and the rest are untouched. {e}",
                args.message_ids.len()
            ))
        })?;
        mailbox.db.forget_message(id).map_err(failed)?;
        deleted += 1;
    }

    Ok(json!({ "deleted": deleted }))
}

async fn undo(mailboxes: Arc<Mailboxes>, args: Undo) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let action = mailbox.db.new_action().map_err(failed)?;
    let undone = mailbox.db.undo(&args.undo_id, &action).map_err(failed)?;
    if undone > 0 {
        send_soon(&mailbox);
    }

    Ok(json!({ "undone": undone }))
}

async fn outbox(mailboxes: Arc<Mailboxes>) -> Result<Value, CommandError> {
    let mut accounts = Vec::new();
    for account in mailboxes.accounts().map_err(failed)? {
        let mailbox = mailboxes.open(&account).await.map_err(failed)?;
        let (queued, refused) = mailbox.db.outbox().map_err(failed)?;
        let refused: Vec<Value> = refused
            .iter()
            .map(|op| {
                json!({
                    "message_id": op.message_id,
                    "subject": op.subject,
                    "change": match op.kind.as_str() {
                        "is_read" => "read or unread",
                        "flag_status" => "flag",
                        "folder_id" => "folder",
                        "categories" => "categories",
                        other => other,
                    },
                    "error": op.error,
                })
            })
            .collect();
        let sending = mailbox.db.queued_drafts().map_err(failed)?.len();
        accounts.push(json!({ "account_id": account.id, "queued": queued, "sending": sending, "refused": refused }));
    }

    Ok(json!({ "accounts": accounts }))
}

async fn dismiss(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    mailbox.db.dismiss_failed_ops().map_err(failed)?;

    Ok(json!({ "dismissed": true }))
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

fn message_ids() -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" },
        "minItems": 1,
        "maxItems": MOST,
        "description": "Message ids, from mail.messages.list or mail.search."
    })
}

fn account_id() -> Value {
    json!({ "type": "string", "description": "Which account, from accounts.list. May be left out when there is only one." })
}

fn edited() -> Value {
    json!({
        "type": "object",
        "properties": {
            "changed": { "type": "integer", "description": "How many messages were not already as asked, and were changed." },
            "undo_id": { "type": "string", "description": "Give this to mail.undo to take the action back." }
        },
        "required": ["changed", "undo_id"]
    })
}

const HOW: &str = "The change shows on this computer at once and is sent to Microsoft 365 in the background; \
                   if it cannot be sent now it is kept and sent later. app.outbox.status says what is still waiting.";

pub(crate) fn register(
    registry: &mut Registry,
    mailboxes: Arc<Mailboxes>,
) -> Result<(), RegistryError> {
    registry.register(
        CommandSpec {
            id: "mail.messages.set_read".to_owned(),
            title: "Mark as read or unread".to_owned(),
            description: format!("Mark messages as read or as unread. {HOW}"),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "message_ids": message_ids(),
                    "is_read": { "type": "boolean" }
                },
                "required": ["message_ids", "is_read"],
                "additionalProperties": false
            }),
            result_schema: edited(),
            examples: vec![json!({ "message_ids": ["message-000008"], "is_read": true })],
        },
        handler(&mailboxes, |mailboxes, args: SetRead| {
            edit(
                mailboxes,
                args.account_id,
                args.message_ids,
                MessageEdit::Read(args.is_read),
            )
        }),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.messages.set_flag".to_owned(),
            title: "Flag, complete or clear the flag".to_owned(),
            description: format!(
                "Flag messages for follow-up (`flagged`), mark the follow-up done (`complete`), or clear the flag (`notFlagged`). {HOW}"
            ),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "message_ids": message_ids(),
                    "flag": { "enum": ["flagged", "complete", "notFlagged"] }
                },
                "required": ["message_ids", "flag"],
                "additionalProperties": false
            }),
            result_schema: edited(),
            examples: vec![json!({ "message_ids": ["message-000008"], "flag": "flagged" })],
        },
        handler(&mailboxes, |mailboxes, args: SetFlag| {
            edit(mailboxes, args.account_id, args.message_ids, MessageEdit::Flag(args.flag))
        }),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.messages.move".to_owned(),
            title: "Move to a folder".to_owned(),
            description: format!(
                "Move messages to another folder, named by its id from mail.folders.list or, for a standard folder, by its well-known name. \
                 Deleting is a move to `deleteditems`, archiving a move to `archive`, and marking as junk a move to `junkemail`; \
                 nothing is destroyed, and mail.undo moves the messages back. {HOW}"
            ),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "message_ids": message_ids(),
                    "folder_id": { "type": "string", "description": "A folder's id, or inbox, deleteditems, archive, junkemail and so on." }
                },
                "required": ["message_ids", "folder_id"],
                "additionalProperties": false
            }),
            result_schema: edited(),
            examples: vec![json!({ "message_ids": ["message-000008"], "folder_id": "archive" })],
        },
        handler(&mailboxes, move_messages),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.messages.set_categories".to_owned(),
            title: "Set categories".to_owned(),
            description: format!(
                "Give messages exactly these categories, by name, in place of whatever they had; an empty list clears them. \
                 The names the mailbox has are in mail.categories.list. {HOW}"
            ),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "message_ids": message_ids(),
                    "categories": { "type": "array", "items": { "type": "string", "minLength": 1, "maxLength": 255 }, "maxItems": 25, "uniqueItems": true }
                },
                "required": ["message_ids", "categories"],
                "additionalProperties": false
            }),
            result_schema: edited(),
            examples: vec![json!({ "message_ids": ["message-000008"], "categories": ["Clients"] })],
        },
        handler(&mailboxes, |mailboxes, args: SetCategories| {
            edit(mailboxes, args.account_id, args.message_ids, MessageEdit::Categories(args.categories))
        }),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.categories.list".to_owned(),
            title: "List categories".to_owned(),
            description: "The categories this mailbox has, each a name and a colour (`none`, or `preset0` to `preset24`, which mail programs map to colours of their own). \
                          Read from Microsoft 365 when it can be reached, and otherwise as last read."
                .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id() },
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "categories": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": { "name": { "type": "string" }, "color": { "type": "string" } },
                            "required": ["name", "color"]
                        }
                    }
                },
                "required": ["categories"]
            }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, categories),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.messages.delete_permanently".to_owned(),
            title: "Delete permanently".to_owned(),
            description: "Delete messages for good. They leave every folder, Deleted Items included, and cannot be had back from here or from Outlook; \
                          mail.undo does not bring them back. Ordinary deleting is mail.messages.move to `deleteditems`, which can be undone. \
                          This is done at once and needs Microsoft 365 in reach."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id(), "message_ids": message_ids() },
                "required": ["message_ids"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "deleted": { "type": "integer" } },
                "required": ["deleted"]
            }),
            // A message that is not there is already as deleted as it can be.
            examples: vec![json!({ "message_ids": ["no-such-message"] })],
        },
        handler(&mailboxes, purge),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.undo".to_owned(),
            title: "Undo".to_owned(),
            description: "Take back an action by the `undo_id` it gave. Each message it changed goes back to what it was, \
                          unless it has been changed again since. An id that is not known undoes nothing."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id(), "undo_id": { "type": "string" } },
                "required": ["undo_id"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "undone": { "type": "integer", "description": "How many messages went back." } },
                "required": ["undone"]
            }),
            examples: vec![json!({ "undo_id": "no-such-action" })],
        },
        handler(&mailboxes, undo),
    )?;

    registry.register(
        CommandSpec {
            id: "app.outbox.status".to_owned(),
            title: "Outbox status".to_owned(),
            description: "For each account: how many changes made on this computer and how many messages are still waiting to be sent to Microsoft 365, \
                          and the changes it refused, each with the message and the reason. A refused change has been taken back here."
                .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: Some("get_outbox_status".to_owned()),
            args_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "accounts": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "account_id": { "type": "string" },
                                "queued": { "type": "integer" },
                                "sending": { "type": "integer", "description": "Messages waiting to be sent. mail.drafts.list has them." },
                                "refused": {
                                    "type": "array",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "message_id": { "type": "string" },
                                            "subject": { "type": "string" },
                                            "change": { "type": "string" },
                                            "error": { "type": "string" }
                                        },
                                        "required": ["message_id", "subject", "change", "error"]
                                    }
                                }
                            },
                            "required": ["account_id", "queued", "refused"]
                        }
                    }
                },
                "required": ["accounts"]
            }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, |mailboxes, _: serde_json::Map<String, Value>| outbox(mailboxes)),
    )?;

    registry.register(
        CommandSpec {
            id: "app.outbox.dismiss_refused".to_owned(),
            title: "Dismiss refused changes".to_owned(),
            description: "Forget the changes Microsoft 365 refused, once they have been read. Nothing in the mailbox changes."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id() },
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "dismissed": { "type": "boolean" } },
                "required": ["dismissed"]
            }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, dismiss),
    )?;

    Ok(())
}
