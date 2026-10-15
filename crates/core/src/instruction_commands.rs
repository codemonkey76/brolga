//! The commands for mail instructions: the standing list each mailbox
//! holds, and which device carries it out (docs/spec-mail-instructions.md),
//! and for this device's own name.

use std::future::Future;
use std::sync::Arc;

use brolga_commands::{CommandError, CommandKind, CommandSpec, Handler, Registry, RegistryError};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::device::{self, Device};
use crate::instructions::{
    self, ACTIONS, Action, Instruction, LONGEST_NAME, LONGEST_TEXT, List, MOST_INSTRUCTIONS,
    Settings, Trouble,
};
use crate::mailboxes::{Mailbox, Mailboxes};

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

fn this_device(mailboxes: &Mailboxes) -> Result<Device, CommandError> {
    device::this(mailboxes.global()).map_err(failed)
}

fn shown_list(list: &List) -> Value {
    json!({
        "enabled": list.enabled,
        "version": list.version,
        "written_by": list.written_by.as_ref().map(|w| json!({ "device_id": w.device_id, "device_name": w.device_name })),
        "written_at": list.written_at,
        "instructions": list.instructions,
    })
}

fn shown(settings: Option<&Settings>, device: &Device, offline: bool) -> Value {
    let list = settings.map(|s| s.list.clone()).unwrap_or_default();
    let processor = settings.and_then(|s| s.processor.clone());
    let mut shown = shown_list(&list);
    shown["set_up"] = json!(settings.is_some());
    shown["processing_here"] = json!(processor.as_ref().is_some_and(|p| p.device_id == device.id));
    shown["processor"] = json!(processor);
    shown["last_active"] = json!(settings.and_then(|s| s.last_active.clone()));
    shown["this_device"] = json!(device);
    shown["offline"] = json!(offline);

    shown
}

#[derive(Deserialize)]
struct Account {
    account_id: Option<String>,
}

async fn list(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let device = this_device(&mailboxes)?;

    match instructions::read(&mailbox).await {
        Ok(settings) => Ok(shown(settings.as_ref(), &device, false)),
        // Out of reach, they are shown as last read, and said to be so.
        Err(Trouble::Unreached(_)) if instructions::kept(&mailbox).is_some() => {
            Ok(shown(instructions::kept(&mailbox).as_ref(), &device, true))
        }
        Err(trouble) => Err(failed(trouble)),
    }
}

#[derive(Deserialize)]
struct One {
    account_id: Option<String>,
    instruction_id: String,
}

async fn get(mailboxes: Arc<Mailboxes>, args: One) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let settings = instructions::read(&mailbox).await.map_err(failed)?;
    let list = settings.map(|s| s.list).unwrap_or_default();
    let instruction = list
        .instructions
        .iter()
        .find(|each| each.id == args.instruction_id)
        .ok_or_else(|| no_such(&args.instruction_id))?;

    Ok(json!({ "instruction": instruction, "version": list.version }))
}

fn no_such(id: &str) -> CommandError {
    failed(format!(
        "There is no instruction `{id}`. The instructions are in mail.instructions.list."
    ))
}

#[derive(Deserialize)]
struct Set {
    account_id: Option<String>,
    instruction_id: Option<String>,
    name: String,
    text: String,
    enabled: Option<bool>,
    actions: Vec<Action>,
    version: Option<u64>,
}

async fn set(mailboxes: Arc<Mailboxes>, args: Set) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let device = this_device(&mailboxes)?;
    let id = match &args.instruction_id {
        Some(id) => id.clone(),
        None => format!("i-{}", device::random_id(8).map_err(failed)?),
    };
    let mut actions = Vec::new();
    for action in args.actions {
        if !actions.contains(&action) {
            actions.push(action);
        }
    }
    let instruction = Instruction {
        id: id.clone(),
        name: args.name.trim().to_owned(),
        text: args.text.trim().to_owned(),
        enabled: args.enabled.unwrap_or(true),
        actions,
    };
    let existing = args.instruction_id.is_some();
    let made = instruction.clone();
    let list = instructions::change_list(&mailbox, &device, args.version, move |list| {
        match list.instructions.iter_mut().find(|each| each.id == id) {
            Some(each) => *each = made,
            None if existing => {
                return Err(format!(
                    "There is no instruction `{id}`: it may have been deleted on another device. \
                     Leave out instruction_id to add it as a new one."
                ));
            }
            None => list.instructions.push(made),
        }
        Ok(())
    })
    .await
    .map_err(failed)?;

    Ok(json!({ "instruction": instruction, "version": list.version }))
}

#[derive(Deserialize)]
struct Delete {
    account_id: Option<String>,
    instruction_id: String,
    version: Option<u64>,
}

async fn delete(mailboxes: Arc<Mailboxes>, args: Delete) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let device = this_device(&mailboxes)?;
    // One that is not there is as good as deleted, and nothing is set up
    // in the mailbox to say so.
    let there = instructions::read(&mailbox)
        .await
        .map_err(failed)?
        .is_some_and(|s| {
            s.list
                .instructions
                .iter()
                .any(|each| each.id == args.instruction_id)
        });
    if !there {
        return Ok(json!({ "deleted": false, "version": null }));
    }
    let id = args.instruction_id;
    let list = instructions::change_list(&mailbox, &device, args.version, move |list| {
        list.instructions.retain(|each| each.id != id);
        Ok(())
    })
    .await
    .map_err(failed)?;

    Ok(json!({ "deleted": true, "version": list.version }))
}

#[derive(Deserialize)]
struct Reorder {
    account_id: Option<String>,
    instruction_id: String,
    position: usize,
    version: Option<u64>,
}

async fn reorder(mailboxes: Arc<Mailboxes>, args: Reorder) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let device = this_device(&mailboxes)?;
    let id = args.instruction_id;
    let position = args.position;
    let list = instructions::change_list(&mailbox, &device, args.version, move |list| {
        let at = list
            .instructions
            .iter()
            .position(|each| each.id == id)
            .ok_or_else(|| format!("There is no instruction `{id}`."))?;
        let moved = list.instructions.remove(at);
        let to = position.saturating_sub(1).min(list.instructions.len());
        list.instructions.insert(to, moved);
        Ok(())
    })
    .await
    .map_err(failed)?;

    Ok(shown_list(&list))
}

#[derive(Deserialize)]
struct Enable {
    account_id: Option<String>,
    enabled: bool,
    version: Option<u64>,
}

async fn set_enabled(mailboxes: Arc<Mailboxes>, args: Enable) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let device = this_device(&mailboxes)?;
    let enabled = args.enabled;
    let list = instructions::change_list(&mailbox, &device, args.version, move |list| {
        list.enabled = enabled;
        Ok(())
    })
    .await
    .map_err(failed)?;

    Ok(shown_list(&list))
}

async fn take(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let device = this_device(&mailboxes)?;
    let settings = instructions::set_processor(&mailbox, Some(&device))
        .await
        .map_err(failed)?;

    Ok(shown(Some(&settings), &device, false))
}

async fn release(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let device = this_device(&mailboxes)?;
    // With nothing set up, no device has the role to give up.
    if instructions::read(&mailbox)
        .await
        .map_err(failed)?
        .is_none()
    {
        return Ok(shown(None, &device, false));
    }
    let settings = instructions::set_processor(&mailbox, None)
        .await
        .map_err(failed)?;

    Ok(shown(Some(&settings), &device, false))
}

#[derive(Deserialize)]
struct Message {
    account_id: Option<String>,
    message_id: String,
}

async fn processed(mailboxes: Arc<Mailboxes>, args: Message) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let mark = instructions::processed(&mailbox, &args.message_id)
        .await
        .map_err(failed)?
        .ok_or_else(|| failed("There is no such message."))?;

    Ok(json!({ "processed": mark.is_some(), "mark": mark }))
}

async fn device_get(mailboxes: Arc<Mailboxes>, _: Value) -> Result<Value, CommandError> {
    Ok(json!({ "device": this_device(&mailboxes)? }))
}

#[derive(Deserialize)]
struct Rename {
    name: String,
}

async fn device_rename(mailboxes: Arc<Mailboxes>, args: Rename) -> Result<Value, CommandError> {
    let device = device::rename(mailboxes.global(), &args.name).map_err(failed)?;

    Ok(json!({ "device": device }))
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

pub(crate) fn register(
    registry: &mut Registry,
    mailboxes: Arc<Mailboxes>,
) -> Result<(), RegistryError> {
    let account_id = json!({ "type": "string", "description": "Which mailbox, from accounts.list or accounts.shared.list. May be left out when there is only one account." });
    let version = json!({ "type": "integer", "minimum": 0, "description": "The list's version as it was read, from mail.instructions.list. Given, the change is refused if the list has been changed since, on this device or another. Left out, it is made to the list as it is now." });
    let actions = json!({
        "type": "array",
        "items": { "type": "string", "enum": ACTIONS },
        "minItems": 1,
        "uniqueItems": true,
        "description": "What the instruction may do to a message: `move` it to a folder the instruction names, `set_categories` from the mailbox's own list, `flag` it with or without a due day, `set_read` (read or unread). The model can choose nothing else."
    });
    let instruction = json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "name": { "type": "string" },
            "text": { "type": "string", "description": "The instruction, in plain words." },
            "enabled": { "type": "boolean" },
            "actions": actions
        },
        "required": ["id", "name", "text", "enabled", "actions"]
    });
    let device = json!({
        "type": "object",
        "properties": { "id": { "type": "string" }, "name": { "type": "string" } },
        "required": ["id", "name"]
    });
    let writer = json!({
        "type": ["object", "null"],
        "properties": { "device_id": { "type": "string" }, "device_name": { "type": "string" } }
    });
    let list_shown = |more: Value| {
        let mut properties = json!({
            "enabled": { "type": "boolean", "description": "The list as a whole is on." },
            "version": { "type": "integer", "description": "One more each time the list is changed, on any device." },
            "written_by": writer,
            "written_at": { "type": ["string", "null"] },
            "instructions": { "type": "array", "items": instruction, "description": "In the order they are tried." }
        });
        if let (Some(properties), Some(more)) = (properties.as_object_mut(), more.as_object()) {
            properties.extend(more.clone());
        }
        json!({
            "type": "object",
            "properties": properties,
            "required": ["enabled", "version", "instructions"]
        })
    };
    let processor = json!({
        "type": ["object", "null"],
        "description": "The device that carries out this mailbox's instructions. None has been chosen while this is null, and then nothing is done.",
        "properties": {
            "device_id": { "type": "string" },
            "device_name": { "type": "string" },
            "since": { "type": "string" }
        }
    });
    let settings_shown = list_shown(json!({
        "set_up": { "type": "boolean", "description": "Instructions have been written to this mailbox at some time." },
        "processor": processor,
        "processing_here": { "type": "boolean", "description": "This device is the processing device." },
        "last_active": { "type": ["string", "null"], "description": "When the processing device last said it was running. It is written at most every 15 minutes." },
        "this_device": device,
        "offline": { "type": "boolean", "description": "Microsoft 365 could not be reached, and this is the list as last read." }
    }));
    let one = json!({
        "type": "object",
        "properties": { "instruction": instruction, "version": { "type": "integer" } },
        "required": ["instruction", "version"]
    });
    const WHAT: &str = "Mail instructions are a mailbox's standing list of plain-language instructions, which an AI model applies to each message arriving in the Inbox, \
        on the one device chosen to do so. They are kept in the mailbox, so every device signed in to it sees the same list. \
        They are not the server's inbox rules (mail.rules.*).";

    registry.register(
        CommandSpec {
            id: "mail.instructions.list".to_owned(),
            title: "List mail instructions".to_owned(),
            description: format!(
                "The mailbox's instructions in the order they are tried, whether the list is on, its version, and which device carries them out and when it was last active. {WHAT}"
            ),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: Some("list_instructions".to_owned()),
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id },
                "additionalProperties": false
            }),
            result_schema: settings_shown.clone(),
            examples: vec![json!({})],
        },
        handler(&mailboxes, list),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.get".to_owned(),
            title: "Get a mail instruction".to_owned(),
            description: format!(
                "One of the mailbox's instructions, by its id from mail.instructions.list. {WHAT}"
            ),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id, "instruction_id": { "type": "string" } },
                "required": ["instruction_id"],
                "additionalProperties": false
            }),
            result_schema: one.clone(),
            examples: vec![json!({ "instruction_id": "i-newsletters" })],
        },
        handler(&mailboxes, get),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.set".to_owned(),
            title: "Add or change a mail instruction".to_owned(),
            description: format!(
                "Add an instruction to the end of the mailbox's list, or, given an instruction_id, replace that one whole. \
                 An instruction has a name, its text in plain words (at most {LONGEST_TEXT} characters), whether it is on, and what it may do. \
                 A mailbox can have at most {MOST_INSTRUCTIONS} instructions. \
                 Instructions act on mail unattended, so a change asks the person first. {WHAT}"
            ),
            kind: CommandKind::Action,
            // It decides what is done to mail, unseen, from then on.
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id,
                    "instruction_id": { "type": "string", "description": "The instruction to replace. Left out, a new one is added." },
                    "name": { "type": "string", "minLength": 1, "maxLength": LONGEST_NAME },
                    "text": { "type": "string", "minLength": 1, "maxLength": LONGEST_TEXT },
                    "enabled": { "type": "boolean", "description": "On unless this is false." },
                    "actions": actions,
                    "version": version
                },
                "required": ["name", "text", "actions"],
                "additionalProperties": false
            }),
            result_schema: one,
            examples: vec![json!({
                "name": "Telstra invoices",
                "text": "Invoices from Telstra go in Accounts/Telstra and are categorised Bills.",
                "actions": ["move", "set_categories"]
            })],
        },
        handler(&mailboxes, set),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.delete".to_owned(),
            title: "Delete a mail instruction".to_owned(),
            description: format!(
                "Delete one of the mailbox's instructions. What it has already done to mail stays done. To stop it for a while, set it off with mail.instructions.set instead. {WHAT}"
            ),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id, "instruction_id": { "type": "string" }, "version": version },
                "required": ["instruction_id"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "deleted": { "type": "boolean", "description": "False where there was no such instruction." }, "version": { "type": ["integer", "null"] } },
                "required": ["deleted", "version"]
            }),
            examples: vec![json!({ "instruction_id": "i-quotes" })],
        },
        handler(&mailboxes, delete),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.reorder".to_owned(),
            title: "Move a mail instruction in the order".to_owned(),
            description: format!(
                "Move one of the mailbox's instructions to a place in the order, counted from 1; the others keep their order around it. {WHAT}"
            ),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id,
                    "instruction_id": { "type": "string" },
                    "position": { "type": "integer", "minimum": 1, "description": "1 is first. Past the end is last." },
                    "version": version
                },
                "required": ["instruction_id", "position"],
                "additionalProperties": false
            }),
            result_schema: list_shown(json!({})),
            examples: vec![json!({ "instruction_id": "i-newsletters", "position": 1 })],
        },
        handler(&mailboxes, reorder),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.set_enabled".to_owned(),
            title: "Turn mail instructions on or off".to_owned(),
            description: format!(
                "Turn the mailbox's whole list of instructions on or off. Off, none of them is applied to mail, whatever each one's own setting. {WHAT}"
            ),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id, "enabled": { "type": "boolean" }, "version": version },
                "required": ["enabled"],
                "additionalProperties": false
            }),
            result_schema: list_shown(json!({})),
            examples: vec![json!({ "enabled": true })],
        },
        handler(&mailboxes, set_enabled),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.processor.take".to_owned(),
            title: "Process mail instructions on this device".to_owned(),
            description: format!(
                "Make this device the one that carries out the mailbox's instructions, taking the role from any device that has it. \
                 Only one device does so, and no device takes the role by itself. Only the person can choose it, in Brolga's window. {WHAT}"
            ),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id },
                "additionalProperties": false
            }),
            result_schema: settings_shown.clone(),
            examples: vec![json!({})],
        },
        handler(&mailboxes, take),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.processor.release".to_owned(),
            title: "Stop processing mail instructions".to_owned(),
            description: format!(
                "Leave the mailbox with no processing device, whichever device had the role, so that its instructions are carried out nowhere until one is chosen. \
                 Only the person can do this, in Brolga's window. {WHAT}"
            ),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id },
                "additionalProperties": false
            }),
            result_schema: settings_shown,
            examples: vec![json!({})],
        },
        handler(&mailboxes, release),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.processed".to_owned(),
            title: "Has a message been dealt with by the instructions".to_owned(),
            description: format!(
                "Whether a message carries the mark the processing device leaves on each message it has dealt with, and when and by which version of the list. \
                 A marked message is not dealt with again as it arrives. {WHAT}"
            ),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id, "message_id": { "type": "string" } },
                "required": ["message_id"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "processed": { "type": "boolean" }, "mark": { "type": ["object", "null"] } },
                "required": ["processed", "mark"]
            }),
            examples: vec![json!({ "message_id": "message-000008" })],
        },
        handler(&mailboxes, processed),
    )?;

    registry.register(
        CommandSpec {
            id: "app.device.get".to_owned(),
            title: "This device".to_owned(),
            description: "This copy of Brolga as other devices signed in to the same mailboxes see it: its id, made the first time it ran, and its name, which starts as the computer's own."
                .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "additionalProperties": false }),
            result_schema: json!({ "type": "object", "properties": { "device": device }, "required": ["device"] }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, device_get),
    )?;

    registry.register(
        CommandSpec {
            id: "app.device.rename".to_owned(),
            title: "Rename this device".to_owned(),
            description: format!(
                "Give this copy of Brolga another name, the one other devices show it by, for instance as the device that carries out a mailbox's instructions. At most {} characters.",
                device::NAME_LIMIT
            ),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "name": { "type": "string", "minLength": 1, "maxLength": device::NAME_LIMIT } },
                "required": ["name"],
                "additionalProperties": false
            }),
            result_schema: json!({ "type": "object", "properties": { "device": device }, "required": ["device"] }),
            examples: vec![json!({ "name": "Work laptop" })],
        },
        handler(&mailboxes, device_rename),
    )?;

    Ok(())
}
