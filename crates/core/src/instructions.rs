//! Mail instructions: the standing list each mailbox holds, and which
//! device carries it out (docs/spec-mail-instructions.md).
//!
//! They are kept in the mailbox, so that every device signed in to it sees
//! the same: on one message in a hidden folder named `Brolga`, as
//! single-value extended properties in Brolga's own property set ("Where it
//! lives"). That message is a draft, and stays in that folder, which a
//! normal folder list and the folder tree's delta never show.
//!
//! Microsoft 365 is not asked to refuse a change made over another's: an
//! `If-Match` on a message is not documented ("Update message",
//! SOURCES.md). The list carries a version instead. A change says which
//! version it was made to, and is refused if the list has moved on; and
//! after it is written it is read back, so that two devices writing at the
//! same moment find out, and the one whose change did not stay is told so.

use std::collections::HashMap;

use brolga_graph::GraphError;
use serde::{Deserialize, Serialize};

use crate::device::Device;
use crate::mailboxes::Mailbox;

/// The hidden folder the settings are kept in.
pub(crate) const FOLDER: &str = "Brolga";

/// Brolga's own property set, the one its drafts are marked in.
const SET: &str = "{4431b77f-55c6-4e5f-9693-e9ccc2a3f0fb}";

fn property(name: &str) -> String {
    format!("String {SET} Name {name}")
}

/// The properties of the settings message.
fn instructions_property() -> String {
    property("Instructions")
}
fn processor_property() -> String {
    property("Processor")
}
fn last_active_property() -> String {
    property("LastActive")
}

fn model_property() -> String {
    property("Model")
}
fn arrivals_since_property() -> String {
    property("ArrivalsSince")
}

/// The property each message is marked with once it has been dealt with.
pub(crate) fn processed_property() -> String {
    property("Processed")
}

/// The form the list is written in. A list in a later form was written by
/// a newer Brolga, and is not written over by this one.
const FORMAT: u32 = 1;

pub(crate) const MOST_INSTRUCTIONS: usize = 50;
pub(crate) const LONGEST_TEXT: usize = 2_000;
pub(crate) const LONGEST_NAME: usize = 100;

/// How long the processing device goes between saying it is still there.
const ACTIVE_EVERY: jiff::SignedDuration = jiff::SignedDuration::from_mins(15);

/// Where this mailbox's settings folder is, once found, and the settings as
/// last read, for when Microsoft 365 cannot be reached.
const FOLDER_KEPT: &str = "instructions.folder";
const KEPT: &str = "instructions.kept";

/// What an instruction may do to a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    /// Move it to a folder that the instruction names.
    Move,
    /// Give it categories from the mailbox's own list.
    SetCategories,
    /// Flag it, with a due day or without.
    Flag,
    /// Mark it read or unread.
    SetRead,
    /// Make a To Do task, in a list the instruction names or the default
    /// one, linked back to the message.
    CreateTask,
}

pub(crate) const ACTIONS: [&str; 5] = ["move", "set_categories", "flag", "set_read", "create_task"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Instruction {
    pub id: String,
    pub name: String,
    pub text: String,
    pub enabled: bool,
    pub actions: Vec<Action>,
}

/// Who wrote the list last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Writer {
    pub device_id: String,
    pub device_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct List {
    pub format: u32,
    /// One more each time the list is written.
    pub version: u64,
    /// The list as a whole is on.
    pub enabled: bool,
    pub written_by: Option<Writer>,
    pub written_at: Option<String>,
    pub instructions: Vec<Instruction>,
}

impl Default for List {
    fn default() -> Self {
        Self {
            format: FORMAT,
            version: 0,
            enabled: true,
            written_by: None,
            written_at: None,
            instructions: Vec::new(),
        }
    }
}

/// The device that carries out the mailbox's instructions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Processor {
    pub device_id: String,
    pub device_name: String,
    /// When it took the role.
    pub since: String,
}

/// The settings as the mailbox holds them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Settings {
    /// The message they are kept on.
    pub message_id: String,
    pub list: List,
    pub processor: Option<Processor>,
    pub last_active: Option<String>,
    /// Which model the processing device asks. Where to reach it, and the
    /// key for it, stay on that device.
    #[serde(default)]
    pub model: Option<ModelChoice>,
    /// Mail that arrived in the Inbox from this moment on is dealt with as
    /// it arrives; mail from before, only by a run over existing mail.
    #[serde(default)]
    pub arrivals_since: Option<String>,
}

/// The model a mailbox's instructions are put to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ModelChoice {
    /// `anthropic` or `ollama`.
    pub provider: String,
    pub model: String,
    /// The person has agreed that this mailbox's mail is sent to Anthropic.
    /// Without it nothing is.
    #[serde(default)]
    pub cloud_consent: bool,
}

/// What stops the settings being read or written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Trouble {
    #[error(
        "Microsoft 365 could not be reached, so {0}. Instructions are kept in the mailbox and can only be changed while it can be reached."
    )]
    Unreached(&'static str),
    #[error(
        "The instructions were changed on {by} since they were read, so this change was not made. The newer list is shown; make the change again on it."
    )]
    Conflict { by: String },
    #[error(
        "These instructions were written by a newer version of Brolga. Update Brolga on this computer to read or change them."
    )]
    Newer,
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Graph(GraphError),
}

impl Trouble {
    fn from_graph(error: GraphError, then: &'static str) -> Self {
        if error.is_transient() {
            Self::Unreached(then)
        } else {
            Self::Graph(error)
        }
    }
}

fn now() -> String {
    jiff::Timestamp::now()
        .strftime("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// A mailbox that has no server behind it, or only its calendar, holds no
/// instructions.
fn usable(mailbox: &Mailbox) -> Result<(), Trouble> {
    if mailbox.archive {
        return Err(Trouble::Refused(
            "An archive is kept on this computer only, so it has no instructions.".to_owned(),
        ));
    }
    if mailbox.calendar_only {
        return Err(Trouble::Refused(
            "Only this mailbox's calendar is shared, so it has no mail for instructions to act on."
                .to_owned(),
        ));
    }

    Ok(())
}

fn read_list(written: Option<&String>) -> Result<List, Trouble> {
    let Some(written) = written.filter(|w| !w.is_empty()) else {
        return Ok(List::default());
    };
    // A list another Brolga wrote in a form this one does not know is left
    // as it is, rather than read in part and written back without the rest.
    let format = serde_json::from_str::<serde_json::Value>(written)
        .ok()
        .and_then(|value| value["format"].as_u64());
    if format.is_none_or(|format| format > u64::from(FORMAT)) {
        return Err(Trouble::Newer);
    }

    serde_json::from_str(written).map_err(|_| Trouble::Newer)
}

fn read_settings(message_id: &str, held: &HashMap<String, String>) -> Result<Settings, Trouble> {
    Ok(Settings {
        message_id: message_id.to_owned(),
        list: read_list(held.get(&instructions_property()))?,
        processor: held
            .get(&processor_property())
            .filter(|p| !p.is_empty())
            .and_then(|p| serde_json::from_str(p).ok()),
        last_active: held
            .get(&last_active_property())
            .filter(|at| !at.is_empty())
            .cloned(),
        model: held
            .get(&model_property())
            .filter(|m| !m.is_empty())
            .and_then(|m| serde_json::from_str(m).ok()),
        arrivals_since: held
            .get(&arrivals_since_property())
            .filter(|at| !at.is_empty())
            .cloned(),
    })
}

/// The settings folder, where there is one, looked for again if the one
/// last found has gone.
async fn folder(mailbox: &Mailbox) -> Result<Option<String>, GraphError> {
    if let Ok(Some(kept)) = mailbox.db.setting(FOLDER_KEPT)
        && !kept.is_empty()
        && mailbox.graph.mail_folder(&kept).await?.is_some()
    {
        return Ok(Some(kept));
    }
    let found = mailbox.graph.hidden_folder(FOLDER).await?.map(|f| f.id);
    let _ = mailbox
        .db
        .set_setting(FOLDER_KEPT, found.as_deref().unwrap_or_default());

    Ok(found)
}

/// The settings as the mailbox holds them now, or none if they were never
/// written. Should there be two settings messages, which two devices making
/// them at once would leave, the one with the later list is taken, and of
/// two alike the older.
pub(crate) async fn read(mailbox: &Mailbox) -> Result<Option<Settings>, Trouble> {
    usable(mailbox)?;
    let unread = |e| Trouble::from_graph(e, "the instructions could not be read");
    let Some(folder) = folder(mailbox).await.map_err(unread)? else {
        keep(mailbox, None);
        return Ok(None);
    };
    let ids = [
        instructions_property(),
        processor_property(),
        last_active_property(),
        model_property(),
        arrivals_since_property(),
    ];
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut found: Option<(Settings, String)> = None;
    for (message_id, received) in mailbox
        .graph
        .folder_messages_received(&folder)
        .await
        .map_err(unread)?
    {
        let Some(held) = mailbox
            .graph
            .message_properties(&message_id, &ids)
            .await
            .map_err(unread)?
        else {
            continue;
        };
        let settings = read_settings(&message_id, &held)?;
        let better = match &found {
            None => true,
            Some((best, at)) => {
                (settings.list.version, std::cmp::Reverse(&received))
                    > (best.list.version, std::cmp::Reverse(at))
            }
        };
        if better {
            found = Some((settings, received));
        }
    }
    let found = found.map(|(settings, _)| settings);
    keep(mailbox, found.as_ref());

    Ok(found)
}

/// The settings as last read, for when the mailbox cannot be reached.
pub(crate) fn kept(mailbox: &Mailbox) -> Option<Settings> {
    mailbox
        .db
        .setting(KEPT)
        .ok()
        .flatten()
        .and_then(|kept| serde_json::from_str(&kept).ok())
}

fn keep(mailbox: &Mailbox, settings: Option<&Settings>) {
    let kept = settings
        .and_then(|s| serde_json::to_string(s).ok())
        .unwrap_or_default();
    let _ = mailbox.db.set_setting(KEPT, &kept);
}

const EXPLAINED: &str = "Brolga keeps this mailbox's settings for mail instructions on this message: \
    the instructions themselves, and which computer carries them out. It is in a hidden folder to keep it \
    out of the way. Please leave it where it is: deleting it deletes the instructions.";

/// The settings, made in the mailbox first if they were never written.
async fn read_or_make(mailbox: &Mailbox) -> Result<Settings, Trouble> {
    if let Some(settings) = read(mailbox).await? {
        return Ok(settings);
    }
    let unmade = |e| Trouble::from_graph(e, "the instructions could not be set up");
    let folder = match folder(mailbox).await.map_err(unmade)? {
        Some(folder) => folder,
        None => match mailbox.graph.create_hidden_folder(FOLDER).await {
            Ok(made) => made.id,
            // Made by another device a moment ago, or made here and the
            // answer lost: either way it is there to be found.
            Err(error) if error.is_transient() => folder(mailbox)
                .await
                .map_err(unmade)?
                .ok_or_else(|| unmade(error))?,
            // Microsoft 365 has no two folders of one name side by side. If
            // the one there is not hidden, it is the person's own.
            Err(GraphError::Refused { status: 409, .. }) => folder(mailbox)
                .await
                .map_err(unmade)?
                .ok_or_else(|| {
                    Trouble::Refused(format!(
                        "There is a folder of your own called “{FOLDER}” at the top of this mailbox, \
                         so Brolga cannot make the hidden folder it keeps instructions in. Rename that folder and try again."
                    ))
                })?,
            Err(error) => return Err(unmade(error)),
        },
    };
    let _ = mailbox.db.set_setting(FOLDER_KEPT, &folder);
    let empty =
        serde_json::to_string(&List::default()).map_err(|e| Trouble::Refused(e.to_string()))?;
    let made = mailbox
        .graph
        .create_message_in(
            &folder,
            &serde_json::json!({
                "subject": "Brolga settings",
                "body": { "contentType": "text", "content": EXPLAINED },
                "singleValueExtendedProperties": [{ "id": instructions_property(), "value": empty }]
            }),
        )
        .await;
    if let Err(error) = made
        && !error.is_transient()
    {
        return Err(unmade(error));
    }
    // Made, or perhaps made with the answer lost: read back what is there.
    read(mailbox)
        .await?
        .ok_or(Trouble::Unreached("the instructions could not be set up"))
}

/// Change the list: `change` is made to it as it is now, if it is still
/// the version the change was made to (`expected`, where one is given),
/// and the list is written back and read again to see that it stayed.
pub(crate) async fn change_list(
    mailbox: &Mailbox,
    device: &Device,
    expected: Option<u64>,
    change: impl FnOnce(&mut List) -> Result<(), String>,
) -> Result<List, Trouble> {
    let settings = read_or_make(mailbox).await?;
    let mut list = settings.list;
    let by = |list: &List| {
        list.written_by
            .as_ref()
            .map(|w| w.device_name.clone())
            .unwrap_or_else(|| "another device".to_owned())
    };
    if expected.is_some_and(|expected| expected != list.version) {
        return Err(Trouble::Conflict { by: by(&list) });
    }
    change(&mut list).map_err(Trouble::Refused)?;
    check(&list).map_err(Trouble::Refused)?;
    list.format = FORMAT;
    list.version += 1;
    list.written_by = Some(Writer {
        device_id: device.id.clone(),
        device_name: device.name.clone(),
    });
    list.written_at = Some(now());

    let written = serde_json::to_string(&list).map_err(|e| Trouble::Refused(e.to_string()))?;
    let unwritten = |e| Trouble::from_graph(e, "the instructions were not changed");
    mailbox
        .graph
        .set_message_properties(
            &settings.message_id,
            &[(&instructions_property(), &written)],
        )
        .await
        .map_err(unwritten)?;

    let now_held = read(mailbox).await?.ok_or(Trouble::Unreached(
        "the instructions could not be read back",
    ))?;
    if now_held.list != list {
        return Err(Trouble::Conflict {
            by: by(&now_held.list),
        });
    }

    Ok(list)
}

/// A list that may be kept.
fn check(list: &List) -> Result<(), String> {
    if list.instructions.len() > MOST_INSTRUCTIONS {
        return Err(format!(
            "A mailbox can have at most {MOST_INSTRUCTIONS} instructions."
        ));
    }
    for each in &list.instructions {
        let name = each.name.trim();
        if name.is_empty() {
            return Err("An instruction needs a name.".to_owned());
        }
        if name.chars().count() > LONGEST_NAME {
            return Err(format!(
                "An instruction's name can be at most {LONGEST_NAME} characters."
            ));
        }
        if each.text.trim().is_empty() {
            return Err(format!("“{name}” says nothing: give it some words."));
        }
        if each.text.chars().count() > LONGEST_TEXT {
            return Err(format!(
                "“{name}” is too long: an instruction can be at most {LONGEST_TEXT} characters."
            ));
        }
        if each.actions.is_empty() {
            return Err(format!(
                "“{name}” may do nothing: choose at least one thing it may do."
            ));
        }
    }

    Ok(())
}

/// Make this device the one that carries out the mailbox's instructions,
/// or, with none, make it no device at all.
pub(crate) async fn set_processor(
    mailbox: &Mailbox,
    device: Option<&Device>,
) -> Result<Settings, Trouble> {
    let settings = read_or_make(mailbox).await?;
    let at = now();
    let processor = device.map(|device| Processor {
        device_id: device.id.clone(),
        device_name: device.name.clone(),
        since: at.clone(),
    });
    let written = match &processor {
        Some(processor) => {
            serde_json::to_string(processor).map_err(|e| Trouble::Refused(e.to_string()))?
        }
        // A property cannot be taken off a message, only emptied.
        None => String::new(),
    };
    let processor_id = processor_property();
    let active_id = last_active_property();
    let mut given = vec![(processor_id.as_str(), written.as_str())];
    if processor.is_some() {
        given.push((active_id.as_str(), at.as_str()));
    }
    mailbox
        .graph
        .set_message_properties(&settings.message_id, &given)
        .await
        .map_err(|e| Trouble::from_graph(e, "the processing device was not changed"))?;

    let now_held = read(mailbox).await?.ok_or(Trouble::Unreached(
        "the instructions could not be read back",
    ))?;
    if now_held.processor != processor {
        return Err(Trouble::Conflict {
            by: now_held
                .processor
                .map(|p| p.device_name)
                .unwrap_or_else(|| "another device".to_owned()),
        });
    }

    Ok(now_held)
}

/// Choose the model the mailbox's instructions are put to.
pub(crate) async fn set_model(
    mailbox: &Mailbox,
    choice: &ModelChoice,
) -> Result<Settings, Trouble> {
    let settings = read_or_make(mailbox).await?;
    let written = serde_json::to_string(choice).map_err(|e| Trouble::Refused(e.to_string()))?;
    mailbox
        .graph
        .set_message_properties(&settings.message_id, &[(&model_property(), &written)])
        .await
        .map_err(|e| Trouble::from_graph(e, "the model was not changed"))?;
    let now_held = read(mailbox).await?.ok_or(Trouble::Unreached(
        "the instructions could not be read back",
    ))?;
    if now_held.model.as_ref() != Some(choice) {
        return Err(Trouble::Conflict {
            by: "another device".to_owned(),
        });
    }

    Ok(now_held)
}

/// The moment from which arriving mail is dealt with, fixed the first time
/// it is asked for and the same from then on, whichever device asks.
pub(crate) async fn arrivals_since(
    mailbox: &Mailbox,
    settings: &Settings,
) -> Result<String, Trouble> {
    if let Some(since) = &settings.arrivals_since {
        return Ok(since.clone());
    }
    let since = now();
    mailbox
        .graph
        .set_message_properties(
            &settings.message_id,
            &[(&arrivals_since_property(), &since)],
        )
        .await
        .map_err(|e| Trouble::from_graph(e, "the instructions could not be started"))?;
    // Another device that fixed it a moment before wins: what is held is it.
    let held = read(mailbox)
        .await?
        .and_then(|s| s.arrivals_since)
        .unwrap_or(since);

    Ok(held)
}

/// What a sync does for the instructions: read them again, so they can be
/// shown when the mailbox cannot be reached, and, on the processing device,
/// say that it is still there, at most every quarter of an hour. Gives back
/// whether this is the processing device, to deal with the mail that came.
pub(crate) async fn on_sync(mailbox: &Mailbox, device: &Device) -> Result<bool, Trouble> {
    if mailbox.archive || mailbox.calendar_only {
        return Ok(false);
    }
    let Some(settings) = read(mailbox).await? else {
        return Ok(false);
    };
    let mine = settings
        .processor
        .as_ref()
        .is_some_and(|p| p.device_id == device.id);
    let due = settings
        .last_active
        .as_deref()
        .and_then(|at| at.parse::<jiff::Timestamp>().ok())
        .is_none_or(|at| jiff::Timestamp::now().duration_since(at) >= ACTIVE_EVERY);
    if mine && due {
        mailbox
            .graph
            .set_message_properties(&settings.message_id, &[(&last_active_property(), &now())])
            .await
            .map_err(|e| Trouble::from_graph(e, "the processing device's time was not written"))?;
        let _ = read(mailbox).await;
    }

    Ok(mine)
}

/// When a message was dealt with by the instructions, and by which version
/// of them, as the mark it carries says; none if it carries none.
pub(crate) async fn processed(
    mailbox: &Mailbox,
    message_id: &str,
) -> Result<Option<Option<serde_json::Value>>, Trouble> {
    usable(mailbox)?;
    let mark = processed_property();
    let held = mailbox
        .graph
        .message_properties(message_id, &[mark.as_str()])
        .await
        .map_err(|e| Trouble::from_graph(e, "the message could not be read"))?;

    Ok(held.map(|held| {
        held.get(&mark)
            .filter(|value| !value.is_empty())
            .map(|value| {
                serde_json::from_str(value).unwrap_or_else(|_| serde_json::json!({ "raw": value }))
            })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction(name: &str) -> Instruction {
        Instruction {
            id: format!("i-{name}"),
            name: name.to_owned(),
            text: "Invoices from Telstra go in Accounts/Telstra.".to_owned(),
            enabled: true,
            actions: vec![Action::Move],
        }
    }

    #[test]
    fn a_list_beyond_the_limits_is_refused() {
        let mut list = List {
            instructions: (0..MOST_INSTRUCTIONS)
                .map(|n| instruction(&n.to_string()))
                .collect(),
            ..List::default()
        };
        assert!(check(&list).is_ok());
        list.instructions.push(instruction("one too many"));
        assert!(check(&list).is_err());

        let mut long = instruction("long");
        long.text = "x".repeat(LONGEST_TEXT + 1);
        let mut empty = instruction("empty");
        empty.text = "  ".to_owned();
        let mut idle = instruction("idle");
        idle.actions.clear();
        for wrong in [long, empty, idle] {
            let list = List {
                instructions: vec![wrong],
                ..List::default()
            };
            assert!(check(&list).is_err());
        }
    }

    #[test]
    fn a_list_from_a_newer_brolga_is_not_read() {
        assert_eq!(read_list(None).unwrap(), List::default());
        let newer = r#"{"format":2,"version":3,"enabled":true,"instructions":[],"what_is_this":1}"#;
        assert!(matches!(
            read_list(Some(&newer.to_owned())),
            Err(Trouble::Newer)
        ));
        let unknown_action = r#"{"format":1,"version":3,"enabled":true,"written_by":null,"written_at":null,
            "instructions":[{"id":"a","name":"a","text":"a","enabled":true,"actions":["send"]}]}"#;
        assert!(matches!(
            read_list(Some(&unknown_action.to_owned())),
            Err(Trouble::Newer)
        ));
    }
}
