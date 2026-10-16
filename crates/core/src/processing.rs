//! Carrying out a mailbox's instructions on mail as it arrives, on the
//! processing device (docs/spec-mail-instructions.md, "The processing
//! device" and "Models").
//!
//! A pass takes the Inbox's messages that arrived since the instructions
//! were first switched on and have not been dealt with, oldest first. For
//! each it checks the mark in the mailbox, asks the model, carries out what
//! the instructions allow through the same commands the window uses, writes
//! each action to the agent log, and marks the message. A message the model
//! could not be asked about is left unmarked, to be tried again.

use std::sync::Arc;

use brolga_ai::{AiError, Provider};
use brolga_store::{AgentLogEntry, Body, MessageRow};
use serde_json::{Value, json};

use crate::deciding::{Decided, NotDone, Plan, Step};
use crate::device::Device;
use crate::instructions::{self, ModelChoice, Settings, Trouble};
use crate::mailboxes::{Mailbox, Mailboxes};

/// What the agent log calls the instructions, as the client that acted.
pub(crate) const CLIENT: &str = "mail instructions";

/// Where the device keeps the key for Anthropic, and the address of its
/// Ollama server.
pub(crate) const KEY: &str = "anthropic-api-key";
pub(crate) const OLLAMA_ADDRESS: &str = "ollama_address";

/// Messages sent to a model in the cloud in one day, at most, per mailbox
/// ("Cost guard").
pub(crate) const DAILY_CAP: u32 = 500;

/// Messages dealt with in one pass. Those left over wait for the next.
const PER_PASS: u32 = 25;

/// Tries at a message whose answer could not be used, before it is left for
/// the person.
const TRIES: u32 = 5;

/// What the last pass came to, kept in the mailbox's own settings.
const LAST_PASS: &str = "instructions.last_pass";
/// Until when the model's service asked to be left alone.
const PAUSED_UNTIL: &str = "instructions.paused_until";

/// What is needed to deal with the mailbox's mail, or why it cannot be.
pub(crate) enum Readiness {
    Ready { provider: Provider },
    NotReady(String),
}

pub(crate) fn now_secs() -> i64 {
    jiff::Timestamp::now().as_second()
}

pub(crate) fn today() -> (String, String) {
    let zone = jiff::tz::TimeZone::system();
    let now = jiff::Timestamp::now().to_zoned(zone.clone());
    (
        now.date().to_string(),
        zone.iana_name().unwrap_or("local time").to_owned(),
    )
}

/// Whether this device can deal with the mailbox's mail now, and with
/// which model.
pub(crate) fn readiness(mailboxes: &Mailboxes, settings: &Settings, device: &Device) -> Readiness {
    let not = |why: &str| Readiness::NotReady(why.to_owned());
    if !settings
        .processor
        .as_ref()
        .is_some_and(|p| p.device_id == device.id)
    {
        return not("This computer does not carry out this mailbox's instructions.");
    }
    if !settings.list.enabled {
        return not("The instructions are turned off.");
    }
    if !settings.list.instructions.iter().any(|i| i.enabled) {
        return not("No instruction is on.");
    }
    let Some(choice) = &settings.model else {
        return not("No model has been chosen.");
    };
    provider(mailboxes, choice)
}

/// The model to ask, as this device reaches it.
pub(crate) fn provider(mailboxes: &Mailboxes, choice: &ModelChoice) -> Readiness {
    match choice.provider.as_str() {
        "anthropic" => {
            if !choice.cloud_consent {
                return Readiness::NotReady(
                    "Sending this mailbox's mail to Anthropic has not been agreed to.".to_owned(),
                );
            }
            match mailboxes.ai.secrets.get(KEY) {
                Ok(Some(key)) if !key.is_empty() => Readiness::Ready {
                    provider: Provider::Anthropic {
                        base: mailboxes.ai.anthropic_base.clone(),
                        key,
                        model: choice.model.clone(),
                    },
                },
                Ok(_) => Readiness::NotReady(
                    "No Anthropic API key is saved on this computer.".to_owned(),
                ),
                Err(e) => {
                    Readiness::NotReady(format!("The Anthropic API key could not be read: {e}"))
                }
            }
        }
        "ollama" => Readiness::Ready {
            provider: Provider::Ollama {
                base: ollama_address(mailboxes),
                model: choice.model.clone(),
            },
        },
        other => Readiness::NotReady(format!("`{other}` is not a model provider Brolga knows.")),
    }
}

pub(crate) fn ollama_address(mailboxes: &Mailboxes) -> String {
    mailboxes
        .global()
        .setting(OLLAMA_ADDRESS)
        .ok()
        .flatten()
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| brolga_ai::OLLAMA.to_owned())
}

/// What one pass did.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub(crate) struct Pass {
    /// Messages the model was asked about and that were dealt with.
    pub processed: u32,
    /// Messages another device had already dealt with.
    pub already_done: u32,
    /// Messages that could not be dealt with this time.
    pub failed: u32,
    /// Why the pass stopped short, or did nothing.
    pub stopped: Option<String>,
}

/// Start a pass in the background, unless one is running.
pub(crate) fn soon(
    mailboxes: Arc<Mailboxes>,
    account_id: String,
    mailbox: Arc<Mailbox>,
    device: Device,
) {
    tokio::spawn(async move {
        let _ = pass(&mailboxes, &account_id, &mailbox, &device).await;
    });
}

/// Deal with the mail that has arrived. One pass at a time per mailbox: a
/// pass asked for while another runs does nothing.
pub(crate) async fn pass(
    mailboxes: &Mailboxes,
    account_id: &str,
    mailbox: &Arc<Mailbox>,
    device: &Device,
) -> Result<Pass, Trouble> {
    let Ok(_turn) = mailbox.processing.try_lock() else {
        return Ok(Pass {
            stopped: Some("Another pass is running.".to_owned()),
            ..Pass::default()
        });
    };
    let outcome = run(mailboxes, account_id, mailbox, device).await;
    let kept = match &outcome {
        Ok(pass) => json!({ "at": now_secs(), "pass": pass }),
        Err(trouble) => {
            json!({ "at": now_secs(), "pass": Pass { stopped: Some(trouble.to_string()), ..Pass::default() } })
        }
    };
    let _ = mailbox.db.set_setting(LAST_PASS, &kept.to_string());

    outcome
}

fn stopped(why: impl Into<String>) -> Result<Pass, Trouble> {
    Ok(Pass {
        stopped: Some(why.into()),
        ..Pass::default()
    })
}

async fn run(
    mailboxes: &Mailboxes,
    account_id: &str,
    mailbox: &Arc<Mailbox>,
    device: &Device,
) -> Result<Pass, Trouble> {
    if let Some(until) = mailbox
        .db
        .setting(PAUSED_UNTIL)
        .ok()
        .flatten()
        .and_then(|u| u.parse::<i64>().ok())
        .filter(|until| *until > now_secs())
    {
        return stopped(format!(
            "The model's service asked for a pause, until {}.",
            shown_time(until)
        ));
    }
    let Some(settings) = instructions::read(mailbox).await? else {
        return stopped("There are no instructions.");
    };
    let provider = match readiness(mailboxes, &settings, device) {
        Readiness::Ready { provider } => provider,
        Readiness::NotReady(why) => return stopped(why),
    };
    let Some(http) = mailboxes.ai.http.clone() else {
        return stopped("This computer could not set up a connection to the model.");
    };
    let Some(registry) = mailboxes.registry() else {
        return stopped("Brolga is shutting down.");
    };
    let since = instructions::arrivals_since(mailbox, &settings).await?;
    let local = |e: brolga_store::StoreError| Trouble::Refused(e.to_string());
    let Some(inbox) = mailbox.db.folder_id("inbox").map_err(local)? else {
        return stopped("The Inbox has not been synced yet.");
    };
    let waiting = mailbox
        .db
        .arrivals_to_process(&inbox, &since, now_secs(), PER_PASS)
        .map_err(local)?;
    if waiting.is_empty() {
        return Ok(Pass::default());
    }

    let folders = mailbox.db.folders().map_err(local)?;
    let categories = categories(mailbox).await;
    let Some(plan) = Plan::new(&settings.list, &folders, &categories) else {
        return stopped("No instruction is on.");
    };
    let (day, zone) = today();
    let mut pass = Pass::default();

    for message in waiting {
        // Over the day's cap, the rest wait for tomorrow.
        if let Provider::Anthropic { .. } = provider {
            let used = mailbox
                .db
                .instruction_usage(&day, "anthropic")
                .map_err(local)?;
            if used.requests >= DAILY_CAP {
                pass.stopped = Some(format!(
                    "{DAILY_CAP} messages have been sent to Anthropic today, the most for one day. The rest wait until tomorrow."
                ));
                break;
            }
        }
        match one(
            mailboxes, mailbox, account_id, &registry, &http, &provider, &plan, &settings, device,
            &message, &day, &zone,
        )
        .await
        {
            Ok(Outcome::Done) => pass.processed += 1,
            Ok(Outcome::AlreadyDone) => pass.already_done += 1,
            Ok(Outcome::Failed) => pass.failed += 1,
            // The model or the mailbox cannot be reached: the rest wait too.
            Err(why) => {
                pass.stopped = Some(why);
                break;
            }
        }
    }

    Ok(pass)
}

/// The mailbox's own categories, from Microsoft 365 when it can be
/// reached, and otherwise as last kept.
pub(crate) async fn categories(mailbox: &Mailbox) -> Vec<String> {
    if let Ok(list) = mailbox.graph.master_categories().await {
        return list.into_iter().map(|(_, name, _)| name).collect();
    }
    mailbox
        .db
        .setting("categories")
        .ok()
        .flatten()
        .and_then(|kept| serde_json::from_str::<Vec<Value>>(&kept).ok())
        .map(|all| {
            all.iter()
                .filter_map(|c| c["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

enum Outcome {
    Done,
    AlreadyDone,
    Failed,
}

#[allow(clippy::too_many_arguments)]
async fn one(
    mailboxes: &Mailboxes,
    mailbox: &Arc<Mailbox>,
    account_id: &str,
    registry: &brolga_commands::Registry,
    http: &reqwest::Client,
    provider: &Provider,
    plan: &Plan,
    settings: &Settings,
    device: &Device,
    message: &MessageRow,
    day: &str,
    zone: &str,
) -> Result<Outcome, String> {
    let id = &message.id;
    let local = |e: brolga_store::StoreError| e.to_string();

    // Dealt with already, by this device before a restart or by another
    // that wrongly holds the role too ("Once only").
    let mark = instructions::processed_property();
    match mailbox.graph.message_properties(id, &[mark.as_str()]).await {
        Ok(None) => {
            mailbox.db.mark_instruction_done(id).map_err(local)?;
            return Ok(Outcome::AlreadyDone);
        }
        Ok(Some(held)) if held.get(&mark).is_some_and(|m| !m.is_empty()) => {
            mailbox.db.mark_instruction_done(id).map_err(local)?;
            return Ok(Outcome::AlreadyDone);
        }
        Ok(Some(_)) => {}
        Err(e) if e.is_transient() => {
            return Err(format!("Microsoft 365 could not be reached: {e}"));
        }
        Err(e) => return Err(e.to_string()),
    }

    let body = match body_text(mailbox, id).await {
        Ok(body) => body,
        Err(e) => return Err(format!("The message could not be fetched: {e}")),
    };
    let attachments = if message.has_attachments {
        attachment_names(mailbox, id).await
    } else {
        Vec::new()
    };
    let ask = plan.ask(message, &body, &attachments, day, zone);

    let answer = match timed_ask(mailboxes.global(), http, provider, &ask).await {
        Ok(answer) => {
            let _ = mailbox.db.count_instruction_request(
                day,
                provider.name(),
                i64::try_from(answer.input_tokens).unwrap_or(i64::MAX),
                i64::try_from(answer.output_tokens).unwrap_or(i64::MAX),
            );
            answer
        }
        Err(AiError::Busy { retry_after }) => {
            let until = now_secs() + i64::try_from(retry_after.unwrap_or(60)).unwrap_or(60);
            let _ = mailbox.db.set_setting(PAUSED_UNTIL, &until.to_string());
            return Err(format!(
                "The model's service asked for a pause, until {}.",
                shown_time(until)
            ));
        }
        Err(e) if e.is_transient() => return Err(e.to_string()),
        // A wrong key or a model that is not there is the same for every message.
        Err(e @ AiError::Rejected(_)) => return Err(e.to_string()),
        Err(e) => {
            // Refused, or an answer that could not be used: billed, and tried
            // again later, a few times, then left for the person.
            let _ = mailbox
                .db
                .count_instruction_request(day, provider.name(), 0, 0);
            let tries = mailbox
                .db
                .instruction_mark(id)
                .map_err(local)?
                .map_or(0, |m| m.attempts);
            let wait = 300_i64 << tries.min(6);
            mailbox
                .db
                .mark_instruction_failed(id, &e.to_string(), now_secs() + wait, TRIES)
                .map_err(local)?;
            record(
                mailboxes.global(),
                account_id,
                message,
                None,
                provider,
                "failed",
                &e.to_string(),
                json!({}),
            );
            return Ok(Outcome::Failed);
        }
    };

    let (decided, not_done) = plan.decide(&answer.value);
    for each in &not_done {
        refused(mailboxes.global(), account_id, message, provider, each);
    }
    let mut acted = 0;
    // A move last, so that what else is done is done in the Inbox.
    let mut steps: Vec<&Decided> = decided.iter().collect();
    steps.sort_by_key(|d| matches!(d.step, Step::Move { .. }));
    for each in steps {
        let (command, args) = command_for(account_id, message, &each.step);
        let outcome = registry.execute(command, args.clone()).await;
        let (result, detail, undo_id) = match &outcome {
            Ok(done) => ("ok", each.reason.clone(), done["undo_id"].clone()),
            Err(e) => ("failed", e.to_string(), Value::Null),
        };
        if outcome.is_ok() {
            acted += 1;
        }
        record(
            mailboxes.global(),
            account_id,
            message,
            Some(each),
            provider,
            result,
            &detail,
            json!({ "command": command, "args": args, "undo_id": undo_id }),
        );
    }

    let written = json!({
        "at": jiff::Timestamp::now().strftime("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "version": settings.list.version,
        "device_id": device.id,
        "actions": acted,
    })
    .to_string();
    // Marked here whatever happens, so that this device does not do it
    // again; marked in the mailbox, so that no device does.
    mailbox.db.mark_instruction_done(id).map_err(local)?;
    let _ = mailbox
        .graph
        .set_message_properties(id, &[(&mark, &written)])
        .await;

    Ok(Outcome::Done)
}

pub(crate) fn command_for(
    account_id: &str,
    message: &MessageRow,
    step: &Step,
) -> (&'static str, Value) {
    let ids = json!([message.id]);
    match step {
        Step::Move { folder_id, .. } => (
            "mail.messages.move",
            json!({ "account_id": account_id, "message_ids": ids, "folder_id": folder_id }),
        ),
        Step::AddCategories(more) => {
            // Added to those it has; none taken away.
            let mut all: Vec<String> =
                serde_json::from_str(&message.categories).unwrap_or_default();
            for name in more {
                if !all.iter().any(|have| have.eq_ignore_ascii_case(name)) {
                    all.push(name.clone());
                }
            }
            (
                "mail.messages.set_categories",
                json!({ "account_id": account_id, "message_ids": ids, "categories": all }),
            )
        }
        Step::Flag(due) => {
            let mut args =
                json!({ "account_id": account_id, "message_ids": ids, "flag": "flagged" });
            if let Some(due) = due {
                args["due"] = json!(due);
            }
            ("mail.messages.set_flag", args)
        }
        Step::Read(read) => (
            "mail.messages.set_read",
            json!({ "account_id": account_id, "message_ids": ids, "is_read": read }),
        ),
    }
}

/// Write what was done, or not, to the agent log ("Record").
#[allow(clippy::too_many_arguments)]
pub(crate) fn record(
    global: &brolga_store::GlobalDb,
    account_id: &str,
    message: &MessageRow,
    decided: Option<&Decided>,
    provider: &Provider,
    outcome: &str,
    detail: &str,
    more: Value,
) {
    let mut args = json!({
        "mailbox": account_id,
        "message_id": message.id,
        "subject": message.subject,
        "from": message.from_address,
        "instruction_id": decided.map(|d| d.instruction_id.clone()),
        "instruction": decided.map(|d| d.instruction.clone()),
        "provider": provider.name(),
        "model": provider.model(),
    });
    if let (Some(args), Some(more)) = (args.as_object_mut(), more.as_object()) {
        args.extend(more.clone());
    }
    let command = args["command"]
        .as_str()
        .unwrap_or("mail.instructions.process")
        .to_owned();
    log(
        global,
        &LogLine {
            command,
            args,
            outcome: outcome.to_owned(),
            detail: detail.to_owned(),
        },
    );
}

fn refused(
    global: &brolga_store::GlobalDb,
    account_id: &str,
    message: &MessageRow,
    provider: &Provider,
    each: &NotDone,
) {
    log(
        global,
        &LogLine {
            command: format!("mail.instructions.{}", each.action),
            args: json!({
                "mailbox": account_id,
                "message_id": message.id,
                "subject": message.subject,
                "from": message.from_address,
                "instruction_id": each.instruction_id,
                "provider": provider.name(),
                "model": provider.model(),
            }),
            outcome: "refused".to_owned(),
            detail: each.why.clone(),
        },
    );
}

struct LogLine {
    command: String,
    args: Value,
    outcome: String,
    detail: String,
}

fn log(global: &brolga_store::GlobalDb, line: &LogLine) {
    let _ = global.log_agent_command(&AgentLogEntry {
        at: String::new(),
        client: CLIENT.to_owned(),
        command: line.command.clone(),
        args: line.args.to_string(),
        outcome: line.outcome.clone(),
        detail: line.detail.clone(),
    });
}

/// A message's text: as kept, or fetched and kept.
pub(crate) async fn body_text(mailbox: &Mailbox, id: &str) -> Result<String, String> {
    let body = match mailbox.db.body(id).map_err(|e| e.to_string())? {
        Some(body) => body,
        None => {
            let fetched = mailbox
                .graph
                .message_body(id)
                .await
                .map_err(|e| e.to_string())?;
            let body = Body {
                content_type: fetched.content_type.to_ascii_lowercase(),
                content: fetched.content,
            };
            let text = as_text(&body);
            let _ = mailbox.db.save_body(id, &body, &text);
            return Ok(text);
        }
    };

    Ok(as_text(&body))
}

fn as_text(body: &Body) -> String {
    if body.content_type == "html" {
        brolga_mail::html_to_text(&body.content)
    } else {
        body.content.clone()
    }
}

/// The names of a message's attachments, never their contents.
pub(crate) async fn attachment_names(mailbox: &Mailbox, id: &str) -> Vec<String> {
    if let Ok(Some(kept)) = mailbox.db.attachments(id) {
        return kept.into_iter().map(|a| a.name).collect();
    }
    mailbox
        .graph
        .attachments(id)
        .await
        .map(|all| all.into_iter().filter_map(|a| a.name).collect())
        .unwrap_or_default()
}

fn shown_time(secs: i64) -> String {
    jiff::Timestamp::from_second(secs)
        .map(|at| at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
}

/// Ask, and keep a running average of how long the model takes to
/// answer, for saying how long a run would take.
pub(crate) async fn timed_ask(
    global: &brolga_store::GlobalDb,
    http: &reqwest::Client,
    provider: &Provider,
    ask: &brolga_ai::Ask,
) -> Result<brolga_ai::Answer, AiError> {
    let started = std::time::Instant::now();
    let answer = brolga_ai::ask(http, provider, ask).await;
    if answer.is_ok() {
        let key = speed_key(provider.name(), provider.model());
        let took = started.elapsed().as_secs_f64();
        let average = global
            .setting(&key)
            .ok()
            .flatten()
            .and_then(|kept| kept.parse::<f64>().ok())
            .map_or(took, |kept| kept * 0.8 + took * 0.2);
        let _ = global.set_setting(&key, &format!("{average:.3}"));
    }

    answer
}

fn speed_key(provider: &str, model: &str) -> String {
    format!("instructions.seconds:{provider}:{model}")
}

/// How long the model has taken to answer, on average, in seconds, where
/// it has been asked before.
pub(crate) fn seconds_per_answer(
    global: &brolga_store::GlobalDb,
    provider: &str,
    model: &str,
) -> Option<f64> {
    global
        .setting(&speed_key(provider, model))
        .ok()
        .flatten()
        .and_then(|kept| kept.parse().ok())
}

/// Where the mailbox's processing stands, for the window and for agents.
pub(crate) fn status(mailbox: &Mailbox, provider: &str) -> Value {
    let (day, _) = today();
    let (waiting, failed) = mailbox.db.instruction_backlog().unwrap_or_default();
    let usage = mailbox
        .db
        .instruction_usage(&day, provider)
        .unwrap_or_default();
    let last_error = mailbox.db.last_instruction_error().ok().flatten();
    let last_pass: Value = mailbox
        .db
        .setting(LAST_PASS)
        .ok()
        .flatten()
        .and_then(|kept| serde_json::from_str(&kept).ok())
        .unwrap_or(Value::Null);

    json!({
        "waiting": waiting,
        "failed": failed,
        "today": { "requests": usage.requests, "input_tokens": usage.input_tokens, "output_tokens": usage.output_tokens },
        "daily_cap": DAILY_CAP,
        "last_error": last_error.map(|(message_id, error, at)| json!({ "message_id": message_id, "error": error, "at": shown_time(at) })),
        "last_pass": last_pass.get("pass").cloned().unwrap_or(Value::Null),
        "last_pass_at": last_pass["at"].as_i64().map(shown_time),
    })
}
