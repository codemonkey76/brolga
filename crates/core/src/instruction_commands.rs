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
    ModelChoice, Settings, Trouble,
};
use crate::mailboxes::{Mailbox, Mailboxes};
use crate::processing;
use crate::runs;

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

fn shown_model(mailboxes: &Mailboxes, settings: Option<&Settings>) -> Value {
    let choice = settings.and_then(|s| s.model.clone());
    json!({
        "provider": choice.as_ref().map(|c| c.provider.clone()),
        "model": choice.as_ref().map(|c| c.model.clone()),
        "cloud_consent": choice.as_ref().is_some_and(|c| c.cloud_consent),
        "key_saved": matches!(mailboxes.ai.secrets.get(processing::KEY), Ok(Some(key)) if !key.is_empty()),
        "ollama_address": processing::ollama_address(mailboxes),
        "default_anthropic_model": brolga_ai::DEFAULT_ANTHROPIC_MODEL,
    })
}

async fn model_get(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let settings = match instructions::read(&mailbox).await {
        Ok(settings) => settings,
        Err(Trouble::Unreached(_)) => instructions::kept(&mailbox),
        Err(trouble) => return Err(failed(trouble)),
    };

    Ok(shown_model(&mailboxes, settings.as_ref()))
}

#[derive(Deserialize)]
struct ModelSet {
    account_id: Option<String>,
    provider: String,
    model: Option<String>,
    cloud_consent: Option<bool>,
    ollama_address: Option<String>,
}

async fn model_set(mailboxes: Arc<Mailboxes>, args: ModelSet) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    if let Some(address) = &args.ollama_address {
        let checked = brolga_ai::checked_ollama(address).map_err(failed)?;
        mailboxes
            .global()
            .set_setting(
                processing::OLLAMA_ADDRESS,
                checked.as_str().trim_end_matches('/'),
            )
            .map_err(failed)?;
    }
    let before = instructions::read(&mailbox).await.map_err(failed)?;
    let kept_consent = before
        .as_ref()
        .and_then(|s| s.model.as_ref())
        .is_some_and(|m| m.provider == "anthropic" && m.cloud_consent);
    let model = match (
        args.provider.as_str(),
        args.model.map(|m| m.trim().to_owned()),
    ) {
        ("anthropic", Some(model)) if !model.is_empty() => model,
        ("anthropic", _) => brolga_ai::DEFAULT_ANTHROPIC_MODEL.to_owned(),
        ("ollama", Some(model)) if !model.is_empty() => model,
        ("ollama", _) => {
            return Err(failed(
                "Choose one of the Ollama server's models, from mail.instructions.model.ollama_models.",
            ));
        }
        (other, _) => {
            return Err(failed(format!(
                "`{other}` is not a model provider: choose anthropic or ollama."
            )));
        }
    };
    let choice = ModelChoice {
        cloud_consent: args.provider == "anthropic" && args.cloud_consent.unwrap_or(kept_consent),
        provider: args.provider,
        model,
    };
    let settings = instructions::set_model(&mailbox, &choice)
        .await
        .map_err(failed)?;

    Ok(shown_model(&mailboxes, Some(&settings)))
}

#[derive(Deserialize)]
struct Key {
    key: String,
}

async fn set_key(mailboxes: Arc<Mailboxes>, args: Key) -> Result<Value, CommandError> {
    let key = args.key.trim();
    if key.is_empty() {
        mailboxes
            .ai
            .secrets
            .delete(processing::KEY)
            .map_err(failed)?;
    } else {
        mailboxes
            .ai
            .secrets
            .set(processing::KEY, key)
            .map_err(failed)?;
    }

    Ok(json!({ "key_saved": !key.is_empty() }))
}

#[derive(Deserialize)]
struct Address {
    address: Option<String>,
}

async fn ollama_models(mailboxes: Arc<Mailboxes>, args: Address) -> Result<Value, CommandError> {
    let address = args
        .address
        .unwrap_or_else(|| processing::ollama_address(&mailboxes));
    let http = mailboxes
        .ai
        .http
        .clone()
        .ok_or_else(|| failed("No connection could be set up."))?;
    // A server that is not running is said so, not failed on: it is often
    // just not started yet.
    Ok(match brolga_ai::ollama_models(&http, &address).await {
        Ok(models) => {
            json!({ "address": address, "reachable": true, "models": models, "error": null })
        }
        Err(e) => {
            json!({ "address": address, "reachable": false, "models": [], "error": e.to_string() })
        }
    })
}

/// Ask the mailbox's model about a made-up message, to see that it answers
/// in the form asked for. No mail is sent.
async fn model_test(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let settings = instructions::read(&mailbox).await.map_err(failed)?;
    let mut choice = settings
        .and_then(|s| s.model)
        .ok_or_else(|| failed("No model has been chosen for this mailbox."))?;
    // What goes is made up here, so it needs no leave to go.
    choice.cloud_consent = true;
    let provider = match processing::provider(&mailboxes, &choice) {
        processing::Readiness::Ready { provider } => provider,
        processing::Readiness::NotReady(why) => return Err(failed(why)),
    };
    let http = mailboxes
        .ai
        .http
        .clone()
        .ok_or_else(|| failed("No connection could be set up."))?;
    let list = List {
        instructions: vec![Instruction {
            id: "i-test".to_owned(),
            name: "Newsletters".to_owned(),
            text: "Newsletters are marked read.".to_owned(),
            enabled: true,
            actions: vec![Action::SetRead],
        }],
        ..List::default()
    };
    let plan = crate::deciding::Plan::new(&list, &[], &[])
        .ok_or_else(|| failed("The test could not be set up."))?;
    let ask = plan.ask(
        &crate::deciding::sample(),
        "This week's newsletter: what is new.",
        &[],
        "2026-10-16",
        "UTC",
    );
    let started = std::time::Instant::now();
    match processing::timed_ask(mailboxes.global(), &http, &provider, &ask).await {
        Ok(answer) => Ok(json!({
            "ok": true, "provider": provider.name(), "model": answer.model,
            "seconds": started.elapsed().as_secs_f64(), "answer": answer.value, "error": null
        })),
        Err(e) => Ok(json!({
            "ok": false, "provider": provider.name(), "model": provider.model(),
            "seconds": started.elapsed().as_secs_f64(), "answer": null, "error": e.to_string()
        })),
    }
}

async fn process(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let account = mailboxes
        .account(args.account_id.as_deref())
        .map_err(failed)?;
    let mailbox = mailboxes.open(&account).await.map_err(failed)?;
    let device = this_device(&mailboxes)?;
    let pass = processing::pass(&mailboxes, &account.id, &mailbox, &device)
        .await
        .map_err(failed)?;

    Ok(json!(pass))
}

async fn status(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let device = this_device(&mailboxes)?;
    let settings = match instructions::read(&mailbox).await {
        Ok(settings) => settings,
        Err(Trouble::Unreached(_)) => instructions::kept(&mailbox),
        Err(trouble) => return Err(failed(trouble)),
    };
    let readiness = match &settings {
        Some(settings) => processing::readiness(&mailboxes, settings, &device),
        None => processing::Readiness::NotReady("There are no instructions.".to_owned()),
    };
    let provider = settings
        .as_ref()
        .and_then(|s| s.model.as_ref())
        .map_or("anthropic", |m| {
            if m.provider == "ollama" {
                "ollama"
            } else {
                "anthropic"
            }
        });
    let mut shown = processing::status(&mailbox, provider);
    match readiness {
        processing::Readiness::Ready { .. } => {
            shown["ready"] = json!(true);
            shown["not_ready_because"] = Value::Null;
        }
        processing::Readiness::NotReady(why) => {
            shown["ready"] = json!(false);
            shown["not_ready_because"] = json!(why);
        }
    }

    Ok(shown)
}

#[derive(Deserialize)]
struct Log {
    account_id: Option<String>,
    limit: Option<u32>,
}

async fn log(mailboxes: Arc<Mailboxes>, args: Log) -> Result<Value, CommandError> {
    let account = mailboxes
        .account(args.account_id.as_deref())
        .map_err(failed)?;
    let limit = args.limit.unwrap_or(50).min(500) as usize;
    let entries: Vec<Value> = mailboxes
        .global()
        .agent_log(5_000)
        .map_err(failed)?
        .into_iter()
        .filter(|entry| entry.client == processing::CLIENT)
        .filter_map(|entry| {
            let args: Value = serde_json::from_str(&entry.args).ok()?;
            (args["mailbox"] == account.id.as_str()).then(|| {
                json!({
                    "at": entry.at,
                    "command": entry.command,
                    "outcome": entry.outcome,
                    "detail": entry.detail,
                    "message_id": args["message_id"],
                    "subject": args["subject"],
                    "from": args["from"],
                    "instruction_id": args["instruction_id"],
                    "instruction": args["instruction"],
                    "model": args["model"],
                    "undo_id": args["undo_id"],
                })
            })
        })
        .take(limit)
        .collect();

    Ok(json!({ "entries": entries }))
}

#[derive(Deserialize)]
struct RunScope {
    account_id: Option<String>,
    folder_id: String,
    subfolders: Option<bool>,
    received_from: Option<String>,
    received_to: Option<String>,
}

impl RunScope {
    fn scope(&self) -> Result<runs::Scope, CommandError> {
        for day in [&self.received_from, &self.received_to]
            .into_iter()
            .flatten()
        {
            if day.parse::<jiff::civil::Date>().is_err() {
                return Err(failed(format!(
                    "`{day}` is not a day, written as 2026-10-20."
                )));
            }
        }
        Ok(runs::Scope {
            folder_id: self.folder_id.clone(),
            subfolders: self.subfolders.unwrap_or(false),
            from: self.received_from.clone(),
            to: self.received_to.clone(),
        })
    }
}

async fn run_estimate(mailboxes: Arc<Mailboxes>, args: RunScope) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    runs::estimate(&mailboxes, &mailbox, &args.scope()?)
        .await
        .map_err(failed)
}

async fn run_preview(mailboxes: Arc<Mailboxes>, args: RunScope) -> Result<Value, CommandError> {
    let account = mailboxes
        .account(args.account_id.as_deref())
        .map_err(failed)?;
    let mailbox = mailboxes.open(&account).await.map_err(failed)?;
    let device = this_device(&mailboxes)?;
    let run = runs::preview(&mailboxes, &account.id, &mailbox, &device, &args.scope()?)
        .await
        .map_err(failed)?;

    Ok(json!({ "run": runs::shown(&mailbox, &run) }))
}

#[derive(Deserialize)]
struct RunId {
    account_id: Option<String>,
    run_id: Option<String>,
}

async fn run_status(mailboxes: Arc<Mailboxes>, args: RunId) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let run = match &args.run_id {
        Some(id) => mailbox.db.run(id).map_err(failed)?,
        None => mailbox.db.latest_run().map_err(failed)?,
    };

    Ok(json!({ "run": run.map(|run| runs::shown(&mailbox, &run)) }))
}

#[derive(Deserialize)]
struct RunItems {
    account_id: Option<String>,
    run_id: String,
    filter: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
}

async fn run_items(mailboxes: Arc<Mailboxes>, args: RunItems) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    runs::items(
        &mailbox,
        &args.run_id,
        args.filter.as_deref(),
        args.offset.unwrap_or(0),
        args.limit.unwrap_or(50).min(500),
    )
    .map_err(failed)
}

#[derive(Deserialize)]
struct RunOne {
    account_id: Option<String>,
    run_id: String,
}

async fn run_apply(mailboxes: Arc<Mailboxes>, args: RunOne) -> Result<Value, CommandError> {
    let account = mailboxes
        .account(args.account_id.as_deref())
        .map_err(failed)?;
    let mailbox = mailboxes.open(&account).await.map_err(failed)?;
    let device = this_device(&mailboxes)?;
    let run =
        runs::apply(&mailboxes, &account.id, &mailbox, &device, &args.run_id).map_err(failed)?;

    Ok(json!({ "run": runs::shown(&mailbox, &run) }))
}

async fn run_cancel(mailboxes: Arc<Mailboxes>, args: RunOne) -> Result<Value, CommandError> {
    let mailbox = open(&mailboxes, args.account_id.as_deref()).await?;
    let run = runs::cancel(&mailbox, &args.run_id).map_err(failed)?;

    Ok(json!({ "run": runs::shown(&mailbox, &run) }))
}

async fn run_undo(mailboxes: Arc<Mailboxes>, args: RunOne) -> Result<Value, CommandError> {
    let account = mailboxes
        .account(args.account_id.as_deref())
        .map_err(failed)?;
    let mailbox = mailboxes.open(&account).await.map_err(failed)?;
    runs::undo(&mailboxes, &account.id, &mailbox, &args.run_id)
        .await
        .map_err(failed)
}

/// A key of the right shape for the example, which works nowhere.
fn brolga_fake_key() -> &'static str {
    "sk-ant-test-key"
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

    let model_shown = json!({
        "type": "object",
        "properties": {
            "provider": { "type": ["string", "null"], "enum": ["anthropic", "ollama", null] },
            "model": { "type": ["string", "null"] },
            "cloud_consent": { "type": "boolean", "description": "The person has agreed that this mailbox's mail is sent to Anthropic. Without it nothing is." },
            "key_saved": { "type": "boolean", "description": "An Anthropic API key is saved on this computer. It is never given out." },
            "ollama_address": { "type": "string", "description": "Where this computer reaches Ollama." },
            "default_anthropic_model": { "type": "string" }
        },
        "required": ["provider", "model", "cloud_consent", "key_saved", "ollama_address", "default_anthropic_model"]
    });
    const MODEL: &str = "The model is chosen for each mailbox and kept in it; the API key and the Ollama server's address stay on the computer that carries the instructions out.";

    registry.register(
        CommandSpec {
            id: "mail.instructions.model.get".to_owned(),
            title: "The model mail instructions use".to_owned(),
            description: format!("Which model the mailbox's instructions are put to, at Anthropic or on an Ollama server, whether sending its mail to Anthropic has been agreed to, and whether this computer has a key saved. {MODEL}"),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": { "account_id": account_id }, "additionalProperties": false }),
            result_schema: model_shown.clone(),
            examples: vec![json!({})],
        },
        handler(&mailboxes, model_get),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.model.set".to_owned(),
            title: "Choose the model mail instructions use".to_owned(),
            description: format!(
                "Choose the model the mailbox's instructions are put to: `anthropic` (default model {}) or `ollama` (one of the server's models).                  Only the person can agree to the mailbox's mail being sent to Anthropic (`cloud_consent`) or change where Ollama is reached. {MODEL}",
                brolga_ai::DEFAULT_ANTHROPIC_MODEL
            ),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id,
                    "provider": { "type": "string", "enum": ["anthropic", "ollama"] },
                    "model": { "type": "string", "minLength": 1, "maxLength": 200 },
                    "cloud_consent": { "type": "boolean", "description": "Agree that this mailbox's mail is sent to Anthropic. Left out, it stays as it was." },
                    "ollama_address": { "type": "string", "description": "Where this computer reaches Ollama: on this computer over http, or anywhere over https." }
                },
                "required": ["provider"],
                "additionalProperties": false
            }),
            result_schema: model_shown,
            examples: vec![json!({ "provider": "anthropic" })],
        },
        handler(&mailboxes, model_set),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.model.set_key".to_owned(),
            title: "Save the Anthropic API key".to_owned(),
            description: "Save this computer's Anthropic API key in the system's secret store, or forget it with an empty key. It is never shown or given out again, and goes only to Anthropic. Only the person can do this, in Brolga's window.".to_owned(),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": { "key": { "type": "string", "maxLength": 500 } }, "required": ["key"], "additionalProperties": false }),
            result_schema: json!({ "type": "object", "properties": { "key_saved": { "type": "boolean" } }, "required": ["key_saved"] }),
            examples: vec![json!({ "key": brolga_fake_key() })],
        },
        handler(&mailboxes, set_key),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.model.ollama_models".to_owned(),
            title: "List an Ollama server's models".to_owned(),
            description: "The models an Ollama server has, at this computer's Ollama address or another given. A server that cannot be reached is said so, with why.".to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": { "address": { "type": "string" } }, "additionalProperties": false }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "address": { "type": "string" }, "reachable": { "type": "boolean" },
                    "models": { "type": "array", "items": { "type": "string" } }, "error": { "type": ["string", "null"] }
                },
                "required": ["address", "reachable", "models", "error"]
            }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, ollama_models),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.model.test".to_owned(),
            title: "Try the mail instructions' model".to_owned(),
            description: "Ask the mailbox's model about a made-up message, to see that it can be reached and answers in the form instructions need. No mail is sent. Says how long it took, or why it failed.".to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": { "account_id": account_id }, "additionalProperties": false }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" }, "provider": { "type": "string" }, "model": { "type": "string" },
                    "seconds": { "type": "number" }, "answer": {}, "error": { "type": ["string", "null"] }
                },
                "required": ["ok", "provider", "model", "seconds", "error"]
            }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, model_test),
    )?;

    let pass = json!({
        "type": "object",
        "properties": {
            "processed": { "type": "integer" }, "already_done": { "type": "integer" },
            "failed": { "type": "integer" }, "stopped": { "type": ["string", "null"] }
        },
        "required": ["processed", "already_done", "failed", "stopped"]
    });
    registry.register(
        CommandSpec {
            id: "mail.instructions.process".to_owned(),
            title: "Deal with new mail by the instructions now".to_owned(),
            description: format!(
                "On the processing device, deal now with the Inbox's mail that has arrived since the instructions were switched on and has not been dealt with, oldest first,                  as each sync does by itself. Up to 25 messages at a time. Says how many were dealt with, and why it stopped if it did. {WHAT}"
            ),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": { "account_id": account_id }, "additionalProperties": false }),
            result_schema: pass.clone(),
            examples: vec![json!({})],
        },
        handler(&mailboxes, process),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.status".to_owned(),
            title: "How mail instructions are getting on".to_owned(),
            description: format!(
                "Whether this computer can carry out the mailbox's instructions now and, if not, why; how many messages are waiting to be tried again or were given up on;                  how many were sent to the model today against the day's cap; the last trouble; and what the last pass did. {WHAT}"
            ),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": { "account_id": account_id }, "additionalProperties": false }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "ready": { "type": "boolean" }, "not_ready_because": { "type": ["string", "null"] },
                    "waiting": { "type": "integer" }, "failed": { "type": "integer" },
                    "today": { "type": "object" }, "daily_cap": { "type": "integer" },
                    "last_error": { "type": ["object", "null"] }, "last_pass": {}, "last_pass_at": { "type": ["string", "null"] }
                },
                "required": ["ready", "not_ready_because", "waiting", "failed", "today", "daily_cap", "last_error"]
            }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, status),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.log".to_owned(),
            title: "What mail instructions have done".to_owned(),
            description: format!(
                "What the instructions have done to this mailbox's mail, the latest first: each action with its message, instruction and model, whether it was done (`ok`),                  not allowed (`refused`) or did not work (`failed`), and the undo_id to take it back with mail.undo. {WHAT}"
            ),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id, "limit": { "type": "integer", "minimum": 1, "maximum": 500 } },
                "additionalProperties": false
            }),
            result_schema: json!({ "type": "object", "properties": { "entries": { "type": "array", "items": { "type": "object" } } }, "required": ["entries"] }),
            examples: vec![json!({ "limit": 20 })],
        },
        handler(&mailboxes, log),
    )?;

    let scope = |properties: &mut serde_json::Map<String, Value>| {
        properties.insert("folder_id".to_owned(), json!({ "type": "string", "description": "The folder, by id from mail.folders.list or by a well-known name such as inbox or archive." }));
        properties.insert(
            "subfolders".to_owned(),
            json!({ "type": "boolean", "description": "The folders inside it too." }),
        );
        properties.insert("received_from".to_owned(), json!({ "type": "string", "description": "Messages received on or after this day, 2026-01-31." }));
        properties.insert(
            "received_to".to_owned(),
            json!({ "type": "string", "description": "Messages received on or before this day." }),
        );
    };
    let mut scoped = serde_json::Map::new();
    scoped.insert("account_id".to_owned(), account_id.clone());
    scope(&mut scoped);
    let scoped = json!({ "type": "object", "properties": scoped, "required": ["folder_id"], "additionalProperties": false });
    let run = json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "state": { "type": "string", "enum": ["previewing", "previewed", "applying", "applied", "cancelled", "undone"] },
            "total": { "type": "integer" },
            "counts": { "type": "object", "description": "Messages by where each stands: pending, decided (something to do), nothing, failed, applied, skipped (changed since the preview), unapplied." },
            "actions": { "type": "object", "description": "What the preview would do, by action: move, set_categories, flag, set_read." },
            "undo_until": { "type": ["string", "null"] },
            "error": { "type": ["string", "null"] }
        },
        "required": ["id", "state", "total", "counts", "actions"]
    });
    let one_run = json!({ "type": "object", "properties": { "run": run }, "required": ["run"] });
    let run_id = json!({ "type": "string", "description": "From mail.instructions.run.status or .preview." });
    const RUNS: &str = "A run puts the mailbox's instructions to the mail already in a folder: it is previewed first, and nothing is changed until it is applied. \
        It happens on the processing device, and carries on where it was after a restart.";

    registry.register(
        CommandSpec {
            id: "mail.instructions.run.estimate".to_owned(),
            title: "How big a run over existing mail would be".to_owned(),
            description: format!("How many messages a run over a folder would put to the model, and roughly what it would cost at Anthropic and how long it would take, before anything is sent. Folders not yet fetched in full are named. {RUNS}"),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: scoped.clone(),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "messages": { "type": "integer" }, "estimated_cost_usd": { "type": ["number", "null"] },
                    "estimated_minutes": { "type": ["number", "null"] }, "not_synced": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["messages", "estimated_cost_usd", "estimated_minutes", "not_synced"]
            }),
            examples: vec![json!({ "folder_id": "inbox" })],
        },
        handler(&mailboxes, run_estimate),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.run.preview".to_owned(),
            title: "Preview the instructions over existing mail".to_owned(),
            description: format!("Start a run over a folder's mail, as a preview: the model decides what it would do with each message, and nothing is changed. Its progress and what it found are in mail.instructions.run.status and .items. {RUNS}"),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: scoped,
            result_schema: one_run.clone(),
            examples: vec![json!({ "folder_id": "inbox", "received_from": "2026-10-01" })],
        },
        handler(&mailboxes, run_preview),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.run.status".to_owned(),
            title: "Where a run over existing mail stands".to_owned(),
            description: format!("A run's state and progress: how many messages stand where, and what the preview would do, by action. The latest run if none is named. {RUNS}"),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": { "account_id": account_id, "run_id": run_id }, "additionalProperties": false }),
            result_schema: json!({ "type": "object", "properties": { "run": { "type": ["object", "null"] } }, "required": ["run"] }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, run_status),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.run.items".to_owned(),
            title: "The messages in a run over existing mail".to_owned(),
            description: format!("A page of a run's messages in order, each with what was decided for it and where it stands. `filter`: all, changes (those with something to do), or one state. {RUNS}"),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id, "run_id": run_id,
                    "filter": { "type": "string", "enum": ["all", "changes", "pending", "decided", "nothing", "failed", "applied", "skipped", "unapplied"] },
                    "offset": { "type": "integer", "minimum": 0 }, "limit": { "type": "integer", "minimum": 1, "maximum": 500 }
                },
                "required": ["run_id"],
                "additionalProperties": false
            }),
            result_schema: json!({ "type": "object", "properties": { "items": { "type": "array", "items": { "type": "object" } } }, "required": ["items"] }),
            examples: vec![json!({ "run_id": "run-000000000000", "filter": "changes" })],
        },
        handler(&mailboxes, run_items),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.run.apply".to_owned(),
            title: "Apply a run over existing mail".to_owned(),
            description: format!("Carry out a finished preview, as it was shown. A message changed since its preview is skipped and listed. Everything it does can be undone together, for three days, with mail.instructions.run.undo. {RUNS}"),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": { "account_id": account_id, "run_id": run_id }, "required": ["run_id"], "additionalProperties": false }),
            result_schema: one_run.clone(),
            examples: vec![json!({ "run_id": "run-000000000000" })],
        },
        handler(&mailboxes, run_apply),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.run.cancel".to_owned(),
            title: "Cancel a run over existing mail".to_owned(),
            description: format!("Stop a run being previewed or applied, or set aside one that was previewed. What was already done stays done; no message is left half done. {RUNS}"),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": { "account_id": account_id, "run_id": run_id }, "required": ["run_id"], "additionalProperties": false }),
            result_schema: one_run,
            examples: vec![json!({ "run_id": "run-000000000000" })],
        },
        handler(&mailboxes, run_cancel),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.instructions.run.undo".to_owned(),
            title: "Undo a run over existing mail".to_owned(),
            description: format!("Take back everything an applied run did, within three days of its being applied. A message changed since is left as it now is. {RUNS}"),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": { "account_id": account_id, "run_id": run_id }, "required": ["run_id"], "additionalProperties": false }),
            result_schema: json!({ "type": "object", "properties": { "undone": { "type": "integer" } }, "required": ["undone"] }),
            examples: vec![json!({ "run_id": "run-000000000000" })],
        },
        handler(&mailboxes, run_undo),
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
