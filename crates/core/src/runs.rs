//! Running a mailbox's instructions over mail already in it
//! (docs/spec-mail-instructions.md, "Running over existing mail").
//!
//! A run is first a preview: the model is asked about each message and what
//! it decided is kept, and nothing is changed. Applied, the kept decisions
//! are carried out as they were shown, through the same commands the window
//! uses; a message changed since its preview is skipped. A run is kept in
//! the mailbox's database message by message, so a restart carries on where
//! it was, and it is undone as one, for three days.

use std::sync::Arc;

use brolga_ai::{AiError, Provider};
use brolga_store::{MessageRow, RunRow};
use serde_json::{Value, json};

use crate::deciding::{Decided, Plan, Step};
use crate::device::Device;
use crate::instructions;
use crate::mailboxes::{Mailbox, Mailboxes};
use crate::processing::{self, Readiness};

/// How long what a run did can be undone, in seconds.
pub(crate) const UNDO_FOR: i64 = 72 * 3600;

/// Questions asked at once: a few of a service in the cloud, one of a
/// model on this computer or nearby, which answers one at a time anyway.
fn at_once(provider: &Provider) -> usize {
    match provider {
        Provider::Anthropic { .. } => 4,
        Provider::Ollama { .. } => 1,
    }
}

/// What part of a run is about.
#[derive(Debug, Clone)]
pub(crate) struct Scope {
    pub folder_id: String,
    pub subfolders: bool,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Let the instructions make To Do tasks: left out unless asked for.
    pub include_tasks: bool,
    /// Let the instructions call outside tools: left out unless asked for,
    /// and made when the run is applied, which is their approval.
    pub include_tools: bool,
}

/// The folders a run covers by id, and the names of any of them not yet
/// fetched in full, which would leave messages out.
fn folders(mailbox: &Mailbox, scope: &Scope) -> Result<(Vec<String>, Vec<String>), String> {
    let all = mailbox.db.folders().map_err(|e| e.to_string())?;
    let top = all
        .iter()
        .find(|f| {
            f.id == scope.folder_id || f.well_known.as_deref() == Some(scope.folder_id.as_str())
        })
        .ok_or_else(|| format!("There is no folder `{}` in this mailbox.", scope.folder_id))?;
    let mut chosen = vec![top.clone()];
    if scope.subfolders {
        let mut at = 0;
        while at < chosen.len() {
            let parent = chosen[at].id.clone();
            chosen.extend(
                all.iter()
                    .filter(|f| f.parent_id.as_deref() == Some(parent.as_str()))
                    .cloned(),
            );
            at += 1;
        }
    }
    let unsynced = chosen
        .iter()
        .filter(|f| !f.synced)
        .map(|f| f.name.clone())
        .collect();

    Ok((chosen.into_iter().map(|f| f.id).collect(), unsynced))
}

fn messages(mailbox: &Mailbox, scope: &Scope) -> Result<(Vec<String>, Vec<String>), String> {
    let (ids, unsynced) = folders(mailbox, scope)?;
    let messages = mailbox
        .db
        .messages_for_run(&ids, scope.from.as_deref(), scope.to.as_deref())
        .map_err(|e| e.to_string())?;

    Ok((messages, unsynced))
}

/// The price of a model per million tokens, in and out, in US dollars, as
/// Anthropic lists it for prompts of the size instructions send ("Pricing",
/// SOURCES.md). None for a model not listed here.
fn price(model: &str) -> Option<(f64, f64)> {
    match model {
        "claude-haiku-5-5" => Some((0.10, 0.50)),
        "claude-haiku-4-5" => Some((1.0, 5.0)),
        "claude-sonnet-5-5" | "claude-sonnet-5" => Some((2.0, 10.0)),
        "claude-sonnet-4-6" => Some((3.0, 15.0)),
        "claude-opus-5-5" => Some((4.0, 20.0)),
        "claude-opus-5" | "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6" => {
            Some((5.0, 25.0))
        }
        _ => None,
    }
}

/// How many messages a run would cover, and roughly what it would cost and
/// how long it would take, before anything is sent ("Cost guard").
pub(crate) async fn estimate(
    mailboxes: &Mailboxes,
    mailbox: &Mailbox,
    scope: &Scope,
) -> Result<Value, String> {
    let (ids, unsynced) = messages(mailbox, scope)?;
    let settings = instructions::read(mailbox)
        .await
        .map_err(|e| e.to_string())?;
    let choice = settings.as_ref().and_then(|s| s.model.clone());
    let instructions_chars: usize = settings
        .as_ref()
        .map(|s| {
            s.list
                .instructions
                .iter()
                .filter(|i| i.enabled)
                .map(|i| i.text.len() + 200)
                .sum()
        })
        .unwrap_or_default();
    let count = ids.len();
    // A question is the rules, the instructions and the email, its text cut
    // to 8,000 characters: taken as 4,000 on average, at 3.5 characters to
    // a token. The answer, with any thinking first, as 400 tokens.
    let input_tokens = (3_000 + instructions_chars + 4_600) as f64 / 3.5;
    let output_tokens = 400.0;
    let cost = choice
        .as_ref()
        .filter(|c| c.provider == "anthropic")
        .and_then(|c| price(&c.model))
        .map(|(i, o)| {
            let each = input_tokens * i / 1e6 + output_tokens * o / 1e6;
            (each * count as f64 * 100.0).ceil() / 100.0
        });
    let (provider, model) = choice
        .as_ref()
        .map(|c| (c.provider.clone(), c.model.clone()))
        .unwrap_or_default();
    let seconds = processing::seconds_per_answer(mailboxes.global(), &provider, &model).or(
        match provider.as_str() {
            "anthropic" => Some(3.0),
            _ => None,
        },
    );
    let together = if provider == "anthropic" { 4.0 } else { 1.0 };
    let minutes = seconds.map(|s| (s * count as f64 / together / 60.0).ceil());

    Ok(json!({
        "messages": count,
        "provider": choice.as_ref().map(|c| c.provider.clone()),
        "model": choice.as_ref().map(|c| c.model.clone()),
        "estimated_cost_usd": cost,
        "estimated_minutes": minutes,
        "not_synced": unsynced,
    }))
}

/// Start a run's preview, in the background. Refused while another runs,
/// before the folders are fetched in full, or where this device cannot ask
/// the model.
pub(crate) async fn preview(
    mailboxes: &Arc<Mailboxes>,
    account_id: &str,
    mailbox: &Arc<Mailbox>,
    device: &Device,
    scope: &Scope,
) -> Result<RunRow, String> {
    let local = |e: brolga_store::StoreError| e.to_string();
    if let Some(active) = mailbox.db.active_run().map_err(local)? {
        return Err(format!(
            "A run is already {} in this mailbox. Cancel it, or wait for it, first.",
            active.state
        ));
    }
    let settings = instructions::read(mailbox)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("There are no instructions.")?;
    let provider = match processing::readiness(mailboxes, &settings, device) {
        Readiness::Ready { provider } => provider,
        Readiness::NotReady(why) => return Err(why),
    };
    let (ids, unsynced) = messages(mailbox, scope)?;
    if !unsynced.is_empty() {
        return Err(format!(
            "Not every message in {} has been fetched yet, so a run would leave some out. Let the sync finish first.",
            unsynced.join(", ")
        ));
    }
    if ids.is_empty() {
        return Err("There are no messages there to run the instructions over.".to_owned());
    }
    let id = format!("run-{}", crate::device::random_id(6)?);
    mailbox
        .db
        .create_run(
            &id,
            &scope.folder_id,
            scope.subfolders,
            scope.from.as_deref(),
            scope.to.as_deref(),
            provider.name(),
            provider.model(),
            i64::try_from(settings.list.version).unwrap_or(i64::MAX),
            scope.include_tasks,
            scope.include_tools,
            &ids,
        )
        .map_err(local)?;
    soon(
        mailboxes.clone(),
        account_id.to_owned(),
        mailbox.clone(),
        device.clone(),
    );

    mailbox
        .db
        .run(&id)
        .map_err(local)?
        .ok_or_else(|| "The run was not kept.".to_owned())
}

/// Apply a previewed run, in the background.
pub(crate) fn apply(
    mailboxes: &Arc<Mailboxes>,
    account_id: &str,
    mailbox: &Arc<Mailbox>,
    device: &Device,
    run_id: &str,
) -> Result<RunRow, String> {
    let run = the_run(mailbox, run_id)?;
    if run.state != "previewed" {
        return Err(format!(
            "Only a run whose preview is finished can be applied; this one is {}.",
            run.state
        ));
    }
    mailbox
        .db
        .set_run_state(run_id, "applying", "")
        .map_err(|e| e.to_string())?;
    soon(
        mailboxes.clone(),
        account_id.to_owned(),
        mailbox.clone(),
        device.clone(),
    );

    the_run(mailbox, run_id)
}

/// Stop a run. What was done stays done; what was not, is not.
pub(crate) fn cancel(mailbox: &Mailbox, run_id: &str) -> Result<RunRow, String> {
    let run = the_run(mailbox, run_id)?;
    if !["previewing", "previewed", "applying"].contains(&run.state.as_str()) {
        return Err(format!(
            "This run is {}, so there is nothing to cancel.",
            run.state
        ));
    }
    mailbox
        .db
        .set_run_state(run_id, "cancelled", "")
        .map_err(|e| e.to_string())?;

    the_run(mailbox, run_id)
}

/// Undo everything a run did, within three days of its being applied.
/// What has been changed since, on a message, is left as it is.
pub(crate) async fn undo(
    mailboxes: &Mailboxes,
    account_id: &str,
    mailbox: &Mailbox,
    run_id: &str,
) -> Result<Value, String> {
    let run = the_run(mailbox, run_id)?;
    let applied_at = run.applied_at.unwrap_or(run.created_at);
    if !["applied", "cancelled"].contains(&run.state.as_str()) {
        return Err(format!(
            "This run is {}, so there is nothing to undo.",
            run.state
        ));
    }
    if processing::now_secs() > applied_at + UNDO_FOR {
        return Err("A run can be undone for three days after it was applied, and this one's three days are over.".to_owned());
    }
    let registry = mailboxes.registry().ok_or("Brolga is shutting down.")?;
    let mut undone = 0;
    for kept in mailbox.db.run_undo_ids(run_id).map_err(|e| e.to_string())? {
        let (command, on, undo_id) = match kept.strip_prefix("tasks:") {
            Some(task) => ("tasks.undo", processing::owner(account_id), task.to_owned()),
            None => ("mail.undo", account_id, kept),
        };
        if let Ok(done) = registry
            .execute(command, json!({ "account_id": on, "undo_id": undo_id }))
            .await
        {
            undone += done["undone"].as_u64().unwrap_or_default();
        }
    }
    mailbox
        .db
        .set_run_state(run_id, "undone", "")
        .map_err(|e| e.to_string())?;

    // Calls to outside tools are not Brolga's to undo.
    let not_undone = mailbox
        .db
        .run_items(run_id, None, 0, u32::MAX)
        .map_err(|e| e.to_string())?
        .iter()
        .filter(|item| item.state == "applied" && item.decisions.contains("\"call_tool\""))
        .count();

    Ok(json!({ "undone": undone, "calls_not_undone": not_undone }))
}

fn the_run(mailbox: &Mailbox, run_id: &str) -> Result<RunRow, String> {
    mailbox
        .db
        .run(run_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("There is no run `{run_id}`."))
}

/// Carry on with the run that is under way, if there is one and nothing
/// is carrying it on already: after a restart, or when one is started.
pub(crate) fn soon(
    mailboxes: Arc<Mailboxes>,
    account_id: String,
    mailbox: Arc<Mailbox>,
    device: Device,
) {
    tokio::spawn(async move {
        let Ok(_turn) = mailbox.running.try_lock() else {
            return;
        };
        while let Ok(Some(run)) = mailbox.db.active_run() {
            let went = match run.state.as_str() {
                "previewing" => {
                    preview_some(&mailboxes, &account_id, &mailbox, &device, &run).await
                }
                "applying" => apply_some(&mailboxes, &account_id, &mailbox, &device, &run).await,
                _ => Ok(false),
            };
            match went {
                // More to do: on with it, unless it was cancelled meanwhile.
                Ok(true) => {}
                Ok(false) => break,
                // Out of reach: it waits for the next sync.
                Err(why) => {
                    let _ = mailbox.db.set_run_state(&run.id, &run.state, &why);
                    break;
                }
            }
        }
    });
}

/// How the message stood: what a run's apply checks it still does.
fn snapshot(row: &MessageRow) -> String {
    json!([
        row.folder_id,
        row.is_read,
        row.flag_status,
        row.flag_due,
        row.categories
    ])
    .to_string()
}

fn decided_json(each: &Decided) -> Value {
    let mut shown = json!({ "instruction_id": each.instruction_id, "instruction": each.instruction, "reason": each.reason });
    match &each.step {
        Step::Move { folder_id, path } => {
            shown["action"] = json!("move");
            shown["folder_id"] = json!(folder_id);
            shown["folder"] = json!(path);
        }
        Step::AddCategories(names) => {
            shown["action"] = json!("set_categories");
            shown["categories"] = json!(names);
        }
        Step::Flag(due) => {
            shown["action"] = json!("flag");
            shown["due"] = json!(due);
        }
        Step::Read(read) => {
            shown["action"] = json!("set_read");
            shown["read"] = json!(read);
        }
        Step::CreateTask {
            list_id,
            list,
            title,
            due,
            reminder,
        } => {
            shown["action"] = json!("create_task");
            shown["list_id"] = json!(list_id);
            shown["list"] = json!(list);
            shown["title"] = json!(title);
            shown["due"] = json!(due);
            shown["reminder"] = json!(reminder);
        }
        Step::CallTool {
            calls,
            unattended,
            looked,
        } => {
            shown["action"] = json!("call_tool");
            shown["steps"] = json!(calls);
            shown["looked"] = json!(looked);
            shown["unattended"] = json!(unattended);
        }
    }
    shown
}

fn decided_from(kept: &Value) -> Option<Decided> {
    let step = match kept["action"].as_str()? {
        "move" => Step::Move {
            folder_id: kept["folder_id"].as_str()?.to_owned(),
            path: kept["folder"].as_str().unwrap_or_default().to_owned(),
        },
        "set_categories" => {
            Step::AddCategories(serde_json::from_value(kept["categories"].clone()).ok()?)
        }
        "flag" => Step::Flag(kept["due"].as_str().map(str::to_owned)),
        "set_read" => Step::Read(kept["read"] == true),
        "call_tool" => Step::CallTool {
            calls: serde_json::from_value(kept["steps"].clone()).ok()?,
            unattended: kept["unattended"] == true,
            looked: serde_json::from_value(kept["looked"].clone()).unwrap_or_default(),
        },
        "create_task" => Step::CreateTask {
            list_id: kept["list_id"].as_str()?.to_owned(),
            list: kept["list"].as_str().unwrap_or_default().to_owned(),
            title: kept["title"].as_str()?.to_owned(),
            due: kept["due"].as_str().map(str::to_owned),
            reminder: kept["reminder"].as_str().map(str::to_owned),
        },
        _ => return None,
    };

    Some(Decided {
        instruction_id: kept["instruction_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        instruction: kept["instruction"].as_str().unwrap_or_default().to_owned(),
        step,
        reason: kept["reason"].as_str().unwrap_or_default().to_owned(),
    })
}

/// Ask the model about the next few messages. Gives back whether there is
/// more to do.
async fn preview_some(
    mailboxes: &Mailboxes,
    account_id: &str,
    mailbox: &Arc<Mailbox>,
    device: &Device,
    run: &RunRow,
) -> Result<bool, String> {
    let local = |e: brolga_store::StoreError| e.to_string();
    let settings = instructions::read(mailbox)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("There are no instructions.")?;
    let provider = match processing::readiness(mailboxes, &settings, device) {
        Readiness::Ready { provider } => provider,
        Readiness::NotReady(why) => return Err(why),
    };
    let http = mailboxes
        .ai
        .http
        .clone()
        .ok_or("No connection could be set up.")?;
    let folders = mailbox.db.folders().map_err(local)?;
    let categories = processing::categories(mailbox).await;
    let task_lists = if run.include_tasks {
        processing::task_lists(mailboxes, account_id).await
    } else {
        Vec::new()
    };
    let catalogue = if run.include_tools {
        crate::tool_commands::catalogue(mailboxes)
    } else {
        Vec::new()
    };
    let plan = Plan::new(
        &settings.list,
        &folders,
        &categories,
        &task_lists,
        &catalogue,
    )
    .ok_or("No instruction is on.")?;
    let (day, zone) = processing::today();
    let batch = mailbox
        .db
        .run_items_in(
            &run.id,
            "pending",
            u32::try_from(at_once(&provider) * 4).unwrap_or(4),
        )
        .map_err(local)?;
    if batch.is_empty() {
        mailbox
            .db
            .set_run_state(&run.id, "previewed", "")
            .map_err(local)?;
        return Ok(false);
    }

    for chunk in batch.chunks(at_once(&provider)) {
        // Cancelled meanwhile: stop at once.
        if mailbox
            .db
            .run(&run.id)
            .map_err(local)?
            .is_some_and(|r| r.state != "previewing")
        {
            return Ok(false);
        }
        let asked = chunk.iter().map(|message_id| {
            let (mailbox, http, provider, plan, day, zone) =
                (mailbox.clone(), &http, &provider, &plan, &day, &zone);
            async move {
                let Some(row) = mailbox.db.message(message_id).ok().flatten() else {
                    return (
                        message_id.clone(),
                        None,
                        Err(AiError::BadAnswer(String::new())),
                    );
                };
                let body = match processing::body_text(&mailbox, message_id).await {
                    Ok(body) => body,
                    Err(e) => {
                        return (
                            message_id.clone(),
                            Some(row),
                            Err(AiError::Unreached(format!(
                                "The message could not be fetched: {e}"
                            ))),
                        );
                    }
                };
                let attachments = if row.has_attachments {
                    processing::attachment_names(&mailbox, message_id).await
                } else {
                    Vec::new()
                };
                // Lookups only read, so a preview makes them, as new mail's
                // processing does.
                let asked = processing::ask_looking(
                    mailboxes,
                    account_id,
                    http,
                    provider,
                    plan,
                    &row,
                    &body,
                    &attachments,
                    (day, zone),
                    |answer| {
                        let _ = mailbox.db.count_instruction_request(
                            day,
                            &format!("{}:run", provider.name()),
                            i64::try_from(answer.input_tokens).unwrap_or(i64::MAX),
                            i64::try_from(answer.output_tokens).unwrap_or(i64::MAX),
                        );
                    },
                )
                .await;
                (message_id.clone(), Some(row), asked)
            }
        });
        for (message_id, row, answer) in futures::future::join_all(asked).await {
            match (row, answer) {
                (None, _) => {
                    mailbox
                        .db
                        .set_run_item(
                            &run.id,
                            &message_id,
                            "failed",
                            "The message is no longer in the mailbox.",
                            "[]",
                        )
                        .map_err(local)?;
                }
                (Some(row), Ok((answer, looked, lookups_refused))) => {
                    let (decided, mut not_done) = plan.decide(&answer.value, &looked);
                    not_done.extend(lookups_refused);
                    let decisions: Vec<Value> = decided.iter().map(decided_json).collect();
                    let refused: Vec<Value> = not_done
                        .iter()
                        .map(|n| json!({ "instruction_id": n.instruction_id, "action": n.action, "why": n.why }))
                        .collect();
                    mailbox
                        .db
                        .run_item_decided(
                            &run.id,
                            &message_id,
                            &snapshot(&row),
                            &Value::Array(decisions).to_string(),
                            &Value::Array(refused).to_string(),
                        )
                        .map_err(local)?;
                }
                // The model or the mailbox out of reach, or a wrong key: this
                // message waits, and so does the run.
                (Some(_), Err(e)) if e.is_transient() || matches!(e, AiError::Rejected(_)) => {
                    return Err(e.to_string());
                }
                (Some(_), Err(e)) => {
                    mailbox
                        .db
                        .set_run_item(&run.id, &message_id, "failed", &e.to_string(), "[]")
                        .map_err(local)?;
                }
            }
        }
    }

    Ok(true)
}

/// Carry out the next few previewed messages' decisions. Gives back
/// whether there is more to do.
async fn apply_some(
    mailboxes: &Mailboxes,
    account_id: &str,
    mailbox: &Arc<Mailbox>,
    device: &Device,
    run: &RunRow,
) -> Result<bool, String> {
    let local = |e: brolga_store::StoreError| e.to_string();
    let registry = mailboxes.registry().ok_or("Brolga is shutting down.")?;
    let batch = mailbox
        .db
        .run_items_in(&run.id, "decided", 25)
        .map_err(local)?;
    if batch.is_empty() {
        mailbox
            .db
            .set_run_state(&run.id, "applied", "")
            .map_err(local)?;
        return Ok(false);
    }
    let provider = match run.provider.as_str() {
        "anthropic" => Provider::Anthropic {
            base: String::new(),
            key: String::new(),
            model: run.model.clone(),
        },
        _ => Provider::Ollama {
            base: String::new(),
            model: run.model.clone(),
        },
    };
    let mark = instructions::processed_property();

    for message_id in batch {
        if mailbox
            .db
            .run(&run.id)
            .map_err(local)?
            .is_some_and(|r| r.state != "applying")
        {
            return Ok(false);
        }
        let Some(item) = mailbox.db.run_item(&run.id, &message_id).map_err(local)? else {
            continue;
        };
        // Changed since it was previewed: left as it is, and said so.
        let Some(row) = mailbox
            .db
            .message(&message_id)
            .map_err(local)?
            .filter(|row| snapshot(row) == item.snapshot)
        else {
            mailbox
                .db
                .set_run_item(
                    &run.id,
                    &message_id,
                    "skipped",
                    "It was changed, moved or deleted after the preview.",
                    "[]",
                )
                .map_err(local)?;
            continue;
        };
        let decided: Vec<Decided> = serde_json::from_str::<Vec<Value>>(&item.decisions)
            .unwrap_or_default()
            .iter()
            .filter_map(decided_from)
            .collect();
        let mut steps: Vec<&Decided> = decided.iter().collect();
        steps.sort_by_key(|d| matches!(d.step, Step::Move { .. }));
        let mut undo_ids = Vec::new();
        let mut trouble = Vec::new();
        for each in steps {
            // Applying the run is the approval of the calls it shows.
            if let Step::CallTool {
                calls,
                unattended,
                looked,
            } = &each.step
            {
                let proposal = crate::calls::Proposal {
                    message: &row,
                    instruction_id: &each.instruction_id,
                    instruction: &each.instruction,
                    calls,
                    looked,
                    unattended: *unattended,
                    reason: &each.reason,
                    run_id: Some(&run.id),
                };
                match crate::calls::propose(mailboxes, mailbox, account_id, &proposal).await {
                    Ok(state) if state == "done" => {}
                    Ok(state) => {
                        let tools: Vec<&str> = calls.iter().map(|c| c.tool.as_str()).collect();
                        trouble.push(format!(
                            "The calls to {} came to: {state}.",
                            tools.join(", then ")
                        ));
                    }
                    Err(e) => trouble.push(e),
                }
                continue;
            }
            let (command, args) = processing::command_for(account_id, &row, &each.step);
            match registry.execute(command, args.clone()).await {
                Ok(done) => {
                    // Kept with the command that undoes it: a task's is To Do's.
                    if let Some(undo_id) = done["undo_id"].as_str() {
                        undo_ids.push(match processing::undo_command(command) {
                            "tasks.undo" => format!("tasks:{undo_id}"),
                            _ => undo_id.to_owned(),
                        });
                    }
                    processing::record(
                        mailboxes.global(),
                        account_id,
                        &row,
                        Some(each),
                        &provider,
                        "ok",
                        &each.reason,
                        json!({ "command": command, "args": args, "undo_id": done["undo_id"], "run_id": run.id }),
                    );
                }
                Err(e) => {
                    trouble.push(e.to_string());
                    processing::record(
                        mailboxes.global(),
                        account_id,
                        &row,
                        Some(each),
                        &provider,
                        "failed",
                        &e.to_string(),
                        json!({ "command": command, "args": args, "run_id": run.id }),
                    );
                }
            }
        }
        // What undoes them is kept for three days, each where it is kept:
        // a task's with the To Do of the account the mailbox belongs to.
        let until = processing::now_secs() + UNDO_FOR;
        let (tasks, mail): (Vec<String>, Vec<String>) = undo_ids
            .iter()
            .cloned()
            .partition(|id| id.starts_with("tasks:"));
        mailbox.db.keep_undo(&mail, until).map_err(local)?;
        if !tasks.is_empty()
            && let Ok(account) = mailboxes.account(Some(processing::owner(account_id)))
            && let Ok(owner) = mailboxes.open(&account).await
        {
            let tasks: Vec<String> = tasks
                .iter()
                .map(|id| id.trim_start_matches("tasks:").to_owned())
                .collect();
            owner.db.keep_undo(&tasks, until).map_err(local)?;
        }
        let written = json!({
            "at": jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "version": run.list_version,
            "device_id": device.id,
            "run_id": run.id,
            "actions": undo_ids.len(),
        })
        .to_string();
        let _ = mailbox
            .graph
            .set_message_properties(&message_id, &[(&mark, &written)])
            .await;
        let _ = mailbox.db.mark_instruction_done(&message_id);
        let state = if trouble.is_empty() {
            "applied"
        } else {
            "unapplied"
        };
        mailbox
            .db
            .set_run_item(
                &run.id,
                &message_id,
                state,
                &trouble.join(" "),
                &json!(undo_ids).to_string(),
            )
            .map_err(local)?;
    }
    Ok(true)
}

/// A run as commands give it out: where it stands, and what it found.
pub(crate) fn shown(mailbox: &Mailbox, run: &RunRow) -> Value {
    let counts: serde_json::Map<String, Value> = mailbox
        .db
        .run_counts(&run.id)
        .unwrap_or_default()
        .into_iter()
        .map(|(state, n)| (state, json!(n)))
        .collect();
    let total: u64 = counts.values().filter_map(Value::as_u64).sum();
    let mut actions = serde_json::Map::new();
    for kept in mailbox.db.run_decisions(&run.id).unwrap_or_default() {
        for each in serde_json::from_str::<Vec<Value>>(&kept).unwrap_or_default() {
            if let Some(action) = each["action"].as_str() {
                let n = actions
                    .get(action)
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
                actions.insert(action.to_owned(), json!(n + 1));
            }
        }
    }
    let at = |secs: i64| {
        jiff::Timestamp::from_second(secs)
            .map(|t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
            .ok()
    };

    json!({
        "id": run.id,
        "state": run.state,
        "folder_id": run.folder_id,
        "subfolders": run.subfolders,
        "received_from": run.received_from,
        "received_to": run.received_to,
        "provider": run.provider,
        "model": run.model,
        "total": total,
        "counts": counts,
        "actions": actions,
        "created_at": at(run.created_at),
        "applied_at": run.applied_at.and_then(at),
        "undo_until": run.applied_at.and_then(|a| at(a + UNDO_FOR)),
        "error": (!run.error.is_empty()).then(|| run.error.clone()),
    })
}

/// A page of a run's messages, with what was decided for each.
pub(crate) fn items(
    mailbox: &Mailbox,
    run_id: &str,
    filter: Option<&str>,
    offset: u32,
    limit: u32,
) -> Result<Value, String> {
    the_run(mailbox, run_id)?;
    let rows = mailbox
        .db
        .run_items(run_id, filter, offset, limit)
        .map_err(|e| e.to_string())?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            json!({
                "message_id": row.message_id,
                "subject": row.subject,
                "from": { "name": row.from_name, "address": row.from_address },
                "received_at": row.received_at,
                "state": row.state,
                "decisions": serde_json::from_str::<Value>(&row.decisions).unwrap_or(json!([])),
                "refused": serde_json::from_str::<Value>(&row.refused).unwrap_or(json!([])),
                "error": (!row.error.is_empty()).then_some(row.error),
            })
        })
        .collect();

    Ok(json!({ "items": items }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_decision_is_kept_and_read_back_as_it_was() {
        for step in [
            Step::Move {
                folder_id: "f1".to_owned(),
                path: "Accounts/Telstra".to_owned(),
            },
            Step::AddCategories(vec!["Clients".to_owned()]),
            Step::Flag(Some("2026-10-18".to_owned())),
            Step::Flag(None),
            Step::Read(true),
        ] {
            let decided = Decided {
                instruction_id: "i".to_owned(),
                instruction: "I".to_owned(),
                step,
                reason: "r".to_owned(),
            };
            assert_eq!(decided_from(&decided_json(&decided)), Some(decided));
        }
    }

    #[test]
    fn the_cost_of_a_model_not_listed_is_not_guessed() {
        assert_eq!(price("claude-haiku-5-5"), Some((0.10, 0.50)));
        assert_eq!(price("some-new-model"), None);
    }
}
