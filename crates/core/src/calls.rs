//! Calls to outside tools the mail instructions make, and their approval
//! (docs/spec-mail-instructions.md, "Outside tools").
//!
//! A call that asks first is put up in the mailbox, in a hidden folder of
//! its own, as one message with the call on it and its decision, so that
//! every device signed in to the mailbox can see it and the person can
//! approve or reject it from any of them. The processing device keeps its
//! own copy of the call, and makes only that: an approval changes nothing
//! but the decision. Every call is written down before it is made, so that
//! one interrupted is never made again unasked.

use std::sync::Arc;

use brolga_store::{MessageRow, ToolCallRow};
use serde_json::{Value, json};

use crate::device::Device;
use crate::instructions::{self, property};
use crate::mailboxes::{Mailbox, Mailboxes};

/// The hidden folder calls wait for approval in.
pub(crate) const FOLDER: &str = "Brolga approvals";
const FOLDER_KEPT: &str = "approvals.folder";

fn call_property() -> String {
    property("Call")
}
fn decision_property() -> String {
    property("Decision")
}
fn outcome_property() -> String {
    property("Outcome")
}

/// Tries at a call that could not be made, before it is left for the person.
const TRIES: u32 = 3;

/// A call still in hand this long is taken to have been interrupted.
const INTERRUPTED_AFTER: i64 = 600;

/// A decided item is kept in the mailbox this long, then deleted.
const KEEP_DECIDED: i64 = 30 * 24 * 3600;

const EXPLAINED: &str = "Brolga keeps a call to an outside tool here while it waits for approval: what it would call, with what, and for which message. \
    Approve or reject it in Brolga, under Settings, Instructions. Please leave it where it is.";

fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

fn secs() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// A call an instruction's answer asks for.
pub(crate) struct Proposal<'a> {
    pub message: &'a MessageRow,
    pub instruction_id: &'a str,
    pub instruction: &'a str,
    pub server_id: &'a str,
    pub server: &'a str,
    pub tool: &'a str,
    pub inputs: &'a serde_json::Map<String, Value>,
    pub unattended: bool,
    pub reason: &'a str,
    /// A run over existing mail being applied, which is its approval.
    pub run_id: Option<&'a str>,
}

/// The call as it is put up, and kept: all a person needs to judge it.
fn call_json(account_id: &str, p: &Proposal<'_>) -> String {
    json!({
        "mailbox": account_id,
        "message_id": p.message.id,
        "subject": p.message.subject,
        "from": p.message.from_address,
        "instruction_id": p.instruction_id,
        "instruction": p.instruction,
        "server_id": p.server_id,
        "server": p.server,
        "tool": p.tool,
        "inputs": p.inputs,
        "reason": p.reason,
    })
    .to_string()
}

/// Put a call up, or make it where it need not ask. Gives back what
/// became of it: `waiting`, or what making it came to.
pub(crate) async fn propose(
    mailboxes: &Mailboxes,
    mailbox: &Arc<Mailbox>,
    account_id: &str,
    p: &Proposal<'_>,
) -> Result<String, String> {
    let local = |e: brolga_store::StoreError| e.to_string();
    let row = ToolCallRow {
        id: format!("call-{}", crate::device::random_id(8)?),
        approval_id: String::new(),
        message_id: p.message.id.clone(),
        server_id: p.server_id.to_owned(),
        tool: p.tool.to_owned(),
        call: call_json(account_id, p),
        state: "waiting".to_owned(),
        attempts: 0,
        next_try: 0,
        outcome: String::new(),
        run_id: p.run_id.map(str::to_owned),
        changed_at: 0,
    };
    mailbox.db.add_tool_call(&row).map_err(local)?;
    if p.unattended || p.run_id.is_some() {
        return Ok(make(mailboxes, mailbox, &row).await);
    }
    let approval = put_up(mailbox, &row.call, json!({ "state": "waiting" })).await?;
    mailbox
        .db
        .set_tool_call_approval(&row.id, &approval)
        .map_err(local)?;

    Ok("waiting".to_owned())
}

/// The approvals folder, found or made.
async fn folder(mailbox: &Mailbox) -> Result<String, String> {
    if let Ok(Some(kept)) = mailbox.db.setting(FOLDER_KEPT)
        && !kept.is_empty()
        && matches!(mailbox.graph.mail_folder(&kept).await, Ok(Some(_)))
    {
        return Ok(kept);
    }
    let found = match mailbox
        .graph
        .hidden_folder(FOLDER)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(found) => found.id,
        None => match mailbox.graph.create_hidden_folder(FOLDER).await {
            Ok(made) => made.id,
            Err(_) => mailbox
                .graph
                .hidden_folder(FOLDER)
                .await
                .map_err(|e| e.to_string())?
                .map(|f| f.id)
                .ok_or("The folder approvals wait in could not be made.")?,
        },
    };
    let _ = mailbox.db.set_setting(FOLDER_KEPT, &found);

    Ok(found)
}

/// Put a call up for approval in the mailbox. Gives back its item's id.
async fn put_up(mailbox: &Mailbox, call: &str, decision: Value) -> Result<String, String> {
    let folder = folder(mailbox).await?;
    let shown: Value = serde_json::from_str(call).unwrap_or(Value::Null);
    let made = mailbox
        .graph
        .create_message_in(
            &folder,
            &json!({
                "subject": format!("Brolga: {} {} waits for approval", shown["server"].as_str().unwrap_or_default(), shown["tool"].as_str().unwrap_or_default()),
                "body": { "contentType": "text", "content": EXPLAINED },
                "singleValueExtendedProperties": [
                    { "id": call_property(), "value": call },
                    { "id": decision_property(), "value": decision.to_string() }
                ]
            }),
        )
        .await
        .map_err(|e| format!("The call could not be put up for approval: {e}"))?;

    Ok(made.id)
}

/// Make a call, written down first. Gives back what it came to.
async fn make(mailboxes: &Mailboxes, mailbox: &Mailbox, row: &ToolCallRow) -> String {
    let call: Value = serde_json::from_str(&row.call).unwrap_or(Value::Null);
    let say = |state: &str, outcome: &str, next_try: i64| {
        let _ = mailbox.db.set_tool_call(&row.id, state, outcome, next_try);
    };
    let Some(server) = crate::tool_commands::all_servers(mailboxes)
        .into_iter()
        .find(|s| s.id == row.server_id)
    else {
        let why = format!(
            "The server {} is not set up on this computer.",
            call["server"].as_str().unwrap_or_default()
        );
        say("failed", &why, 0);
        record(mailboxes, &call, "failed", &why);
        outcome(mailbox, row, "failed", &why).await;
        return "failed".to_owned();
    };
    if mailbox.db.tool_call_calling(&row.id).is_err() {
        return "failed".to_owned();
    }
    let inputs = call["inputs"].as_object().cloned().unwrap_or_default();
    let (state, text) =
        match brolga_tools::call(mailboxes.ai.secrets.clone(), &server, &row.tool, inputs).await {
            Ok(called) if called.ok => ("done", called.text),
            Ok(called) => ("failed", called.text),
            // Not reached: tried again later, a few times.
            Err(e) if e.is_transient() && row.attempts + 1 < TRIES => {
                let wait = 300_i64 << row.attempts.min(6);
                say("retry", &e.to_string(), secs() + wait);
                return "retry".to_owned();
            }
            Err(e) => ("failed", e.to_string()),
        };
    say(state, &text, 0);
    record(
        mailboxes,
        &call,
        if state == "done" { "ok" } else { "failed" },
        &text,
    );
    outcome(mailbox, row, state, &text).await;

    state.to_owned()
}

/// Write what a call came to on its item in the mailbox, where it has one.
async fn outcome(mailbox: &Mailbox, row: &ToolCallRow, state: &str, text: &str) {
    if row.approval_id.is_empty() {
        return;
    }
    let written = json!({ "state": state, "text": text, "at": now() }).to_string();
    let _ = mailbox
        .graph
        .set_message_properties(&row.approval_id, &[(&outcome_property(), &written)])
        .await;
}

/// Write a call to the agent log.
fn record(mailboxes: &Mailboxes, call: &Value, outcome: &str, detail: &str) {
    let args = json!({
        "mailbox": call["mailbox"], "message_id": call["message_id"], "subject": call["subject"], "from": call["from"],
        "instruction_id": call["instruction_id"], "instruction": call["instruction"],
        "server": call["server"], "tool": call["tool"], "inputs": call["inputs"],
        "undo_command": null,
    });
    let _ = mailboxes
        .global()
        .log_agent_command(&brolga_store::AgentLogEntry {
            at: String::new(),
            client: crate::processing::CLIENT.to_owned(),
            command: "tools.call".to_owned(),
            args: args.to_string(),
            outcome: outcome.to_owned(),
            detail: detail.chars().take(2_000).collect(),
        });
}

/// Whether the instruction that put a call up still names its tool, on,
/// with the inputs it fixes as the call has them.
fn still_allowed(list: &instructions::List, call: &str) -> bool {
    let call: Value = serde_json::from_str(call).unwrap_or(Value::Null);
    let Some(instruction) = list
        .instructions
        .iter()
        .find(|i| call["instruction_id"].as_str() == Some(i.id.as_str()))
    else {
        return false;
    };
    list.enabled
        && instruction.enabled
        && instruction
            .actions
            .contains(&instructions::Action::CallTool)
        && instruction.tools.iter().any(|t| {
            call["server_id"].as_str() == Some(t.server_id.as_str())
                && call["tool"].as_str() == Some(t.tool.as_str())
                && t.fixed
                    .iter()
                    .all(|(k, v)| call["inputs"].get(k) == Some(v))
        })
}

/// One waiting item as the mailbox holds it.
struct Item {
    id: String,
    call: String,
    decision: Value,
    outcome: Value,
}

async fn items(mailbox: &Mailbox) -> Result<Vec<Item>, String> {
    let Some(folder) = (match mailbox
        .db
        .setting(FOLDER_KEPT)
        .ok()
        .flatten()
        .filter(|k| !k.is_empty())
    {
        Some(kept) => Some(kept),
        None => mailbox
            .graph
            .hidden_folder(FOLDER)
            .await
            .map_err(|e| e.to_string())?
            .map(|f| f.id),
    }) else {
        return Ok(Vec::new());
    };
    let ids = match mailbox.graph.folder_message_ids(&folder).await {
        Ok(ids) => ids,
        // The folder kept is gone: none, until one is made again.
        Err(brolga_graph::GraphError::Refused { status: 404, .. }) => {
            let _ = mailbox.db.set_setting(FOLDER_KEPT, "");
            return Ok(Vec::new());
        }
        Err(e) => return Err(e.to_string()),
    };
    let names = [call_property(), decision_property(), outcome_property()];
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut found = Vec::new();
    for id in ids {
        let Some(held) = mailbox
            .graph
            .message_properties(&id, &names)
            .await
            .map_err(|e| e.to_string())?
        else {
            continue;
        };
        let Some(call) = held.get(&call_property()).cloned() else {
            continue;
        };
        let read = |name: String| {
            held.get(&name)
                .and_then(|v| serde_json::from_str(v).ok())
                .unwrap_or(Value::Null)
        };
        found.push(Item {
            id,
            call,
            decision: read(decision_property()),
            outcome: read(outcome_property()),
        });
    }

    Ok(found)
}

/// The calls waiting for approval, and those decided, as the mailbox
/// holds them: for any device to show.
pub(crate) async fn list(mailbox: &Mailbox) -> Result<Vec<Value>, String> {
    let mut all: Vec<Value> = items(mailbox)
        .await?
        .into_iter()
        .map(|item| {
            let call: Value = serde_json::from_str(&item.call).unwrap_or(Value::Null);
            json!({
                "id": item.id,
                "state": item.decision["state"].as_str().unwrap_or("waiting"),
                "note": item.decision["note"],
                "decided_by": item.decision["by"],
                "decided_at": item.decision["at"],
                "subject": call["subject"], "from": call["from"], "message_id": call["message_id"],
                "instruction": call["instruction"], "server": call["server"], "tool": call["tool"],
                "inputs": call["inputs"], "reason": call["reason"],
                "outcome": item.outcome,
            })
        })
        .collect();
    // Those waiting first.
    all.sort_by_key(|v| v["state"] != "waiting");

    Ok(all)
}

/// Approve or reject a waiting call, from any device. Written and read
/// back, so that a call decided on two devices at once is decided once.
pub(crate) async fn decide(
    mailbox: &Mailbox,
    device: &Device,
    approval_id: &str,
    approve: bool,
) -> Result<Value, String> {
    let decision = decision_property();
    let held = mailbox
        .graph
        .message_properties(approval_id, &[decision.as_str()])
        .await
        .map_err(|e| e.to_string())?
        .ok_or("There is no such call waiting.")?;
    let now_held: Value = held
        .get(&decision)
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or(Value::Null);
    if now_held["state"] != "waiting" {
        return Err(format!(
            "This call was already {} on {}.",
            now_held["state"].as_str().unwrap_or("decided"),
            now_held["by"].as_str().unwrap_or("another device")
        ));
    }
    let ours = json!({
        "state": if approve { "approved" } else { "rejected" },
        "by": device.name, "device_id": device.id, "at": now(),
    });
    mailbox
        .graph
        .set_message_properties(approval_id, &[(&decision, &ours.to_string())])
        .await
        .map_err(|e| e.to_string())?;
    let back = mailbox
        .graph
        .message_properties(approval_id, &[decision.as_str()])
        .await
        .map_err(|e| e.to_string())?
        .and_then(|held| {
            held.get(&decision)
                .and_then(|v| serde_json::from_str::<Value>(v).ok())
        });
    if back.as_ref() != Some(&ours) {
        return Err(
            "This call was decided on another device at the same moment, and that decision stands."
                .to_owned(),
        );
    }

    Ok(ours)
}

/// What the processing device does at each sync: make the calls approved
/// since, as they were put up; give up those rejected; try again those that
/// could not be made; ask again about any interrupted; and clear away
/// items decided long ago.
pub(crate) async fn on_sync(mailboxes: &Mailboxes, mailbox: &Mailbox, device: &Device) {
    let Ok(Some(settings)) = instructions::read(mailbox).await else {
        return;
    };
    if !settings
        .processor
        .as_ref()
        .is_some_and(|p| p.device_id == device.id)
    {
        return;
    }
    // Interrupted: it may have been made, so it is not made again unasked.
    for row in mailbox
        .db
        .tool_calls_interrupted(INTERRUPTED_AFTER)
        .unwrap_or_default()
    {
        let note = json!({ "state": "waiting", "note": "This call was interrupted, and may or may not have been made. Check before approving it again." });
        let approval = if row.approval_id.is_empty() {
            put_up(mailbox, &row.call, note).await.ok()
        } else {
            let _ = mailbox
                .graph
                .set_message_properties(
                    &row.approval_id,
                    &[(&decision_property(), &note.to_string())],
                )
                .await;
            Some(row.approval_id.clone())
        };
        if let Some(approval) = approval {
            let _ = mailbox.db.set_tool_call_approval(&row.id, &approval);
            let _ = mailbox
                .db
                .set_tool_call(&row.id, "waiting", "interrupted", 0);
        }
    }
    for row in mailbox.db.tool_calls_in("retry").unwrap_or_default() {
        if row.next_try <= secs() {
            make(mailboxes, mailbox, &row).await;
        }
    }
    let Ok(items) = items(mailbox).await else {
        return;
    };
    for item in items {
        let Ok(Some(row)) = mailbox.db.tool_call_by_approval(&item.id) else {
            continue;
        };
        if row.state != "waiting" {
            // Long decided: cleared away.
            let at = item.decision["at"]
                .as_str()
                .and_then(|at| at.parse::<jiff::Timestamp>().ok());
            if at.is_some_and(|at| secs() - at.as_second() > KEEP_DECIDED) {
                let _ = mailbox.graph.delete_permanently(&item.id).await;
            }
            continue;
        }
        match item.decision["state"].as_str() {
            // Only the call as it was put up is made, and only while the
            // instruction still allows it.
            Some("approved")
                if item.call == row.call && !still_allowed(&settings.list, &row.call) =>
            {
                let why = "The instruction changed after this call was put up, and no longer allows it, so it was not made.";
                let _ = mailbox.db.set_tool_call(&row.id, "changed", why, 0);
                outcome(mailbox, &row, "changed", why).await;
            }
            Some("approved") if item.call == row.call => {
                make(mailboxes, mailbox, &row).await;
            }
            Some("approved") => {
                let why =
                    "The call in the mailbox was changed after it was put up, so it was not made.";
                let _ = mailbox.db.set_tool_call(&row.id, "changed", why, 0);
                outcome(mailbox, &row, "changed", why).await;
            }
            Some("rejected") => {
                let _ = mailbox.db.set_tool_call(&row.id, "rejected", "", 0);
            }
            _ => {}
        }
    }
}
