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
    due: Option<String>,
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

#[derive(Deserialize)]
struct Category {
    account_id: Option<String>,
    name: String,
    color: Option<String>,
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
    if mailbox.archive {
        // Nothing to tell: the change made here is the whole of it.
        mailbox.db.settle_queued_ops().map_err(failed)?;
    } else if changed > 0 {
        send_soon(&mailbox);
    }

    Ok(json!({ "changed": changed, "undo_id": action }))
}

/// `2026-10-20`, and a day there is.
fn is_day(text: &str) -> bool {
    let bytes = text.as_bytes();
    let number =
        |range: std::ops::Range<usize>| text.get(range).and_then(|part| part.parse::<u32>().ok());
    let (Some(_), Some(month), Some(day)) = (number(0..4), number(5..7), number(8..10)) else {
        return false;
    };

    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
}

/// Flag, complete or clear, and give or take away the day it is due. The
/// several changes this can take are one action, undone as one.
async fn set_flag(mailboxes: Arc<Mailboxes>, args: SetFlag) -> Result<Value, CommandError> {
    if let Some(due) = args.due.as_deref().filter(|due| !due.is_empty()) {
        if args.flag != "flagged" {
            return Err(failed("Only a flagged message has a day it is due."));
        }
        if !is_day(due) {
            return Err(failed(format!(
                "`{due}` is not a day, written as 2026-10-20."
            )));
        }
    }
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let action = mailbox.db.new_action().map_err(failed)?;
    let queue = |edit: MessageEdit| {
        mailbox
            .db
            .queue_edit(&action, &args.message_ids, &edit)
            .map_err(failed)
    };

    let changed = match (args.flag.as_str(), args.due) {
        // Flagged, leaving the day as it is.
        ("flagged", None) => queue(MessageEdit::Flag("flagged".to_owned()))?,
        // Flagged with no day: the flag is cleared and set afresh, which is
        // how a day is taken off one.
        ("flagged", Some(due)) if due.is_empty() => {
            let cleared = queue(MessageEdit::Flag("notFlagged".to_owned()))?;
            let undated = queue(MessageEdit::FlagDue(String::new()))?;
            cleared
                .max(undated)
                .max(queue(MessageEdit::Flag("flagged".to_owned()))?)
        }
        ("flagged", Some(due)) => {
            let flagged = queue(MessageEdit::Flag("flagged".to_owned()))?;
            flagged.max(queue(MessageEdit::FlagDue(due))?)
        }
        // Complete or cleared: there is no longer a day it is due.
        (status, _) => {
            let changed = queue(MessageEdit::Flag(status.to_owned()))?;
            changed.max(queue(MessageEdit::FlagDue(String::new()))?)
        }
    };
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
            .map(|(_, name, color)| json!({ "name": name, "color": color }))
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

/// Keep the list as the server has just given it, and say it.
async fn categories_now(mailbox: &Mailbox) -> Result<Value, CommandError> {
    let list: Vec<Value> = mailbox
        .graph
        .master_categories()
        .await
        .map_err(failed)?
        .into_iter()
        .map(|(_, name, color)| json!({ "name": name, "color": color }))
        .collect();
    let list = Value::Array(list);
    let _ = mailbox.db.set_setting("categories", &list.to_string());

    Ok(json!({ "categories": list }))
}

fn categories_unreached(error: brolga_graph::GraphError) -> CommandError {
    if error.is_transient() {
        failed(
            "Microsoft 365 could not be reached, so the list of categories was not changed. It is kept there, and can only be changed while it can be.",
        )
    } else {
        failed(error)
    }
}

/// The id Microsoft 365 knows a category by, given its name.
async fn category_id(mailbox: &Mailbox, name: &str) -> Result<Option<String>, CommandError> {
    Ok(mailbox
        .graph
        .master_categories()
        .await
        .map_err(categories_unreached)?
        .into_iter()
        .find(|(_, each, _)| each.eq_ignore_ascii_case(name.trim()))
        .map(|(id, _, _)| id))
}

async fn create_category(mailboxes: Arc<Mailboxes>, args: Category) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let name = args.name.trim();
    if name.is_empty() {
        return Err(failed("A category needs a name."));
    }
    if category_id(&mailbox, name).await?.is_some() {
        return Err(failed(format!("There is already a category `{name}`.")));
    }
    mailbox
        .graph
        .create_category(name, args.color.as_deref().unwrap_or("none"))
        .await
        .map_err(categories_unreached)?;

    categories_now(&mailbox).await
}

async fn colour_category(mailboxes: Arc<Mailboxes>, args: Category) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let id = category_id(&mailbox, &args.name).await?.ok_or_else(|| {
        failed(format!(
            "There is no category `{}`. The mailbox's are in mail.categories.list.",
            args.name
        ))
    })?;
    mailbox
        .graph
        .update_category(&id, args.color.as_deref().unwrap_or("none"))
        .await
        .map_err(categories_unreached)?;

    categories_now(&mailbox).await
}

async fn delete_category(mailboxes: Arc<Mailboxes>, args: Category) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    // One that is not there is as good as deleted.
    if let Some(id) = category_id(&mailbox, &args.name).await? {
        mailbox
            .graph
            .delete_category(&id)
            .await
            .map_err(categories_unreached)?;
    }

    categories_now(&mailbox).await
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
                        "flag_status" | "flag_due" => "flag",
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

/// Send what is queued now, and wait for it. The window asks this when a
/// draft is put aside, so its copy in the mailbox is not minutes behind.
async fn flush(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let drained = mailbox.outbox.drain().await.map_err(failed)?;

    Ok(json!({
        "sent": drained.sent, "waiting": drained.waiting, "refused": drained.refused, "drafts_saved": drained.saved
    }))
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
                    "flag": { "enum": ["flagged", "complete", "notFlagged"] },
                    "due": { "type": "string", "description": "With flagged: the day to follow up by, as 2026-10-20, or an empty string for no day. Left out, the day stays as it is." }
                },
                "required": ["message_ids", "flag"],
                "additionalProperties": false
            }),
            result_schema: edited(),
            examples: vec![
                json!({ "message_ids": ["message-000008"], "flag": "flagged" }),
                json!({ "message_ids": ["message-000008"], "flag": "flagged", "due": "2026-10-20" }),
            ],
        },
        handler(&mailboxes, set_flag),
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

    let colour = json!({
        "type": "string",
        "pattern": "^(none|preset([0-9]|1[0-9]|2[0-4]))$",
        "description": "`none`, or `preset0` to `preset24`: red, orange, brown, yellow, green, teal, olive, blue, purple, cranberry, steel, dark steel, grey, dark grey, black, \
                        then the dark of red, orange, brown, yellow, green, teal, olive, blue, purple and cranberry."
    });
    let listed = json!({
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
    });

    registry.register(
        CommandSpec {
            id: "mail.categories.create".to_owned(),
            title: "Add a category".to_owned(),
            description: "Add a category to the list this mailbox has, with a colour. A name cannot be changed once made. \
                          The list is kept by Microsoft 365, so this works only while it can be reached. Gives the list as it then is."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "name": { "type": "string", "minLength": 1, "maxLength": 255 },
                    "color": colour
                },
                "required": ["name"],
                "additionalProperties": false
            }),
            result_schema: listed.clone(),
            examples: vec![json!({ "name": "Training", "color": "preset4" })],
        },
        handler(&mailboxes, create_category),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.categories.delete".to_owned(),
            title: "Delete a category".to_owned(),
            description: "Take a category off the list this mailbox has. Works only while Microsoft 365 can be reached. Gives the list as it then is."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id(), "name": { "type": "string", "minLength": 1 } },
                "required": ["name"],
                "additionalProperties": false
            }),
            result_schema: listed.clone(),
            examples: vec![json!({ "name": "Training" })],
        },
        handler(&mailboxes, delete_category),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.categories.set_color".to_owned(),
            title: "Change a category's colour".to_owned(),
            description: "Give one of the mailbox's categories another colour, which is all of a category that can be changed. \
                          Works only while Microsoft 365 can be reached. Gives the list as it then is."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id(), "name": { "type": "string", "minLength": 1 }, "color": colour },
                "required": ["name", "color"],
                "additionalProperties": false
            }),
            result_schema: listed,
            examples: vec![json!({ "name": "Clients", "color": "preset22" })],
        },
        handler(&mailboxes, colour_category),
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
            id: "app.outbox.flush".to_owned(),
            title: "Send what is waiting".to_owned(),
            description: "Send now whatever is waiting to go to Microsoft 365, and bring the mailbox's Drafts folder level with the drafts here, \
                          without waiting for the next sync. It does only what was already queued, and says how much of it went."
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
                "properties": {
                    "sent": { "type": "integer" },
                    "waiting": { "type": "integer" },
                    "refused": { "type": "integer" },
                    "drafts_saved": { "type": "integer" }
                },
                "required": ["sent", "waiting", "refused", "drafts_saved"]
            }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, flush),
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
