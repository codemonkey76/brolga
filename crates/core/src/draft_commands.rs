//! The commands for writing and sending mail.
//!
//! A draft is kept on this computer as it is written. Sending one hands it
//! to the outbox, which sends it once and only once, now or when it can.

use std::future::Future;
use std::sync::Arc;

use brolga_commands::{CommandError, CommandKind, CommandSpec, Handler, Registry, RegistryError};
use brolga_store::{Draft, DraftEdit, DraftFile, MessageRow};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mailboxes::{Mailbox, Mailboxes};

/// The most recipients one message may have here. Microsoft 365 has its
/// own limit; this one only keeps a mistake from becoming a mailing.
const MOST_RECIPIENTS: usize = 500;

#[derive(Deserialize, Clone)]
struct Person {
    name: Option<String>,
    address: String,
}

#[derive(Deserialize)]
struct Create {
    account_id: Option<String>,
    kind: Option<String>,
    message_id: Option<String>,
    to: Option<Vec<Person>>,
    cc: Option<Vec<Person>>,
    bcc: Option<Vec<Person>>,
    subject: Option<String>,
    body: Option<String>,
}

#[derive(Deserialize)]
struct Edit {
    account_id: Option<String>,
    draft_id: String,
    to: Option<Vec<Person>>,
    cc: Option<Vec<Person>>,
    bcc: Option<Vec<Person>>,
    subject: Option<String>,
    body: Option<String>,
}

#[derive(Deserialize)]
struct One {
    account_id: Option<String>,
    draft_id: String,
}

#[derive(Deserialize)]
struct AddFile {
    account_id: Option<String>,
    draft_id: String,
    /// A file on this computer to attach.
    path: Option<String>,
    /// Or the file itself: its name and its contents in base64.
    name: Option<String>,
    content_base64: Option<String>,
}

#[derive(Deserialize)]
struct RemoveFile {
    account_id: Option<String>,
    draft_id: String,
    file_id: i64,
}

/// The largest file that can be attached yet. Graph takes a file of 3 MB or
/// more only through an upload session, which is not built.
const LARGEST_FILE: usize = 3 * 1024 * 1024 - 1;
/// The most a message's files may come to. Microsoft 365's own default
/// limit for a whole message is 35 MB, and base64 makes files a third bigger.
const MOST_FILES_BYTES: i64 = 25 * 1024 * 1024;

/// What kind of file a name says it is, for the commonest kinds. Anything
/// else is sent as plain bytes, which every mail program accepts.
fn content_type(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "zip" => "application/zip",
        "json" => "application/json",
        "xml" => "application/xml",
        "ics" => "text/calendar",
        "eml" => "message/rfc822",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

fn file_shown(file: &DraftFile) -> Value {
    json!({ "id": file.id, "name": file.name, "content_type": file.content_type, "size": file.size })
}

#[derive(Deserialize)]
struct SetSignature {
    account_id: Option<String>,
    text: String,
    on_replies: Option<bool>,
}

/// The longest signature kept.
const LONGEST_SIGNATURE: usize = 4_000;

/// The account's signature and whether answers get it too.
fn signature(mailbox: &Mailbox) -> Result<(String, bool), CommandError> {
    let text = mailbox
        .db
        .setting("signature")
        .map_err(failed)?
        .unwrap_or_default();
    let on_replies = mailbox
        .db
        .setting("signature_on_replies")
        .map_err(failed)?
        .as_deref()
        == Some("1");

    Ok((text, on_replies))
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
) -> Result<(Arc<Mailbox>, String), CommandError> {
    let account = mailboxes.account(account_id).map_err(failed)?;
    let mailbox = mailboxes.open(&account).await.map_err(failed)?;

    Ok((mailbox, account.username))
}

/// Recipients as they are kept: a JSON array of `{name, address}`, each
/// address trimmed and checked to be one, with no address given twice.
fn kept(people: Option<Vec<Person>>) -> Result<Option<String>, CommandError> {
    let Some(people) = people else {
        return Ok(None);
    };
    if people.len() > MOST_RECIPIENTS {
        return Err(failed(format!(
            "A message here can have at most {MOST_RECIPIENTS} recipients on a line."
        )));
    }

    let mut list: Vec<Value> = Vec::new();
    for person in people {
        let address = person.address.trim().to_owned();
        if !is_address(&address) {
            return Err(failed(format!("`{address}` is not an email address.")));
        }
        if list.iter().any(|each| {
            each["address"]
                .as_str()
                .is_some_and(|a| a.eq_ignore_ascii_case(&address))
        }) {
            continue;
        }
        let name = person
            .name
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty());
        list.push(json!({ "name": name, "address": address }));
    }

    Ok(Some(Value::Array(list).to_string()))
}

/// Enough to catch a slip of the keyboard. Whether the address exists is
/// for the server to say.
fn is_address(text: &str) -> bool {
    let Some((local, domain)) = text.rsplit_once('@') else {
        return false;
    };

    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !text.contains(char::is_whitespace)
        && !text.contains(['<', '>', ',', ';'])
}

fn shown(mailbox: &Mailbox, draft: &Draft) -> Value {
    let files: Vec<Value> = mailbox
        .db
        .draft_files(&draft.id)
        .unwrap_or_default()
        .iter()
        .map(file_shown)
        .collect();
    let list = |raw: &str| serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!([]));

    json!({
        "id": draft.id,
        "kind": draft.kind,
        "message_id": draft.source_id,
        "to": list(&draft.to_recipients),
        "cc": list(&draft.cc_recipients),
        "bcc": list(&draft.bcc_recipients),
        "subject": draft.subject,
        "body": draft.body,
        "state": draft.state,
        "error": draft.error,
        "updated_at": draft.updated_at,
        "files": files,
    })
}

/// `RE: ` or `FW: ` before a subject that does not already begin with it.
fn prefixed(prefix: &str, subject: &str) -> String {
    if subject
        .trim_start()
        .to_ascii_lowercase()
        .starts_with(&prefix.to_ascii_lowercase())
    {
        subject.to_owned()
    } else {
        format!("{prefix} {subject}")
    }
}

/// Who an answer goes to: whoever wrote the message, and for a reply to
/// all, everyone else it went to but the person answering.
fn answer_to(source: &MessageRow, everyone: bool, me: &str) -> (String, String) {
    let parse = |raw: &str| serde_json::from_str::<Vec<Value>>(raw).unwrap_or_default();
    let is_me = |each: &Value| {
        each["address"]
            .as_str()
            .is_some_and(|a| a.eq_ignore_ascii_case(me))
    };
    let author = json!({ "name": source.from_name, "address": source.from_address });

    let mut to = vec![author.clone()];
    let mut cc = Vec::new();
    if everyone {
        let known = |list: &[Value], each: &Value| {
            list.iter().any(|other| {
                other["address"]
                    .as_str()
                    .unwrap_or_default()
                    .eq_ignore_ascii_case(each["address"].as_str().unwrap_or_default())
            })
        };
        for each in parse(&source.to_recipients) {
            if !is_me(&each) && !known(&to, &each) {
                to.push(each);
            }
        }
        for each in parse(&source.cc_recipients) {
            if !is_me(&each) && !known(&to, &each) && !known(&cc, &each) {
                cc.push(each);
            }
        }
    }

    (Value::Array(to).to_string(), Value::Array(cc).to_string())
}

async fn create(mailboxes: Arc<Mailboxes>, args: Create) -> Result<Value, CommandError> {
    let (mailbox, me) = open(&mailboxes, args.account_id.as_deref()).await?;
    let kind = args.kind.as_deref().unwrap_or("new");
    let mut start = DraftEdit {
        to_recipients: kept(args.to)?,
        cc_recipients: kept(args.cc)?,
        bcc_recipients: kept(args.bcc)?,
        subject: args.subject,
        body: args.body,
    };

    if kind != "new" {
        let id = args
            .message_id
            .as_deref()
            .ok_or_else(|| failed("Say which message to answer or forward, with message_id."))?;
        let source = mailbox
            .db
            .message(id)
            .map_err(failed)?
            .ok_or_else(|| failed(format!("There is no message `{id}` on this computer.")))?;

        if kind != "forward" {
            let (to, cc) = answer_to(&source, kind == "reply_all", &me);
            start.to_recipients.get_or_insert(to);
            start.cc_recipients.get_or_insert(cc);
        }
        let prefix = if kind == "forward" { "FW:" } else { "RE:" };
        start
            .subject
            .get_or_insert_with(|| prefixed(prefix, &source.subject));
    }

    // A message started with no words of its own starts with the
    // signature, two lines down, where the writing will go above it.
    if start.body.is_none() {
        let (text, on_replies) = signature(&mailbox)?;
        if !text.trim().is_empty() && (kind == "new" || on_replies) {
            start.body = Some(format!("\n\n{text}"));
        }
    }

    let draft = mailbox
        .db
        .create_draft(
            kind,
            args.message_id.as_deref().filter(|_| kind != "new"),
            &start,
        )
        .map_err(failed)?;

    Ok(json!({ "draft": shown(&mailbox, &draft) }))
}

async fn add_file(mailboxes: Arc<Mailboxes>, args: AddFile) -> Result<Value, CommandError> {
    use base64::Engine;

    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;
    let (name, data) = match (args.path, args.name, args.content_base64) {
        (Some(path), None, None) => {
            let path = std::path::PathBuf::from(path);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .ok_or_else(|| failed("That is not a file."))?;
            // Looked at before it is read, so a huge file is refused, not loaded.
            let size = std::fs::metadata(&path)
                .map_err(|e| failed(format!("`{name}` could not be read. {e}")))?
                .len();
            if size > LARGEST_FILE as u64 {
                return Err(too_large(&name));
            }
            let reading = path.clone();
            let data = tokio::task::spawn_blocking(move || std::fs::read(reading))
                .await
                .map_err(failed)?
                .map_err(|e| failed(format!("`{name}` could not be read. {e}")))?;
            (name, data)
        }
        (None, Some(name), Some(content)) => {
            let data = base64::engine::general_purpose::STANDARD
                .decode(content.trim())
                .map_err(|_| failed("content_base64 is not base64."))?;
            (name, data)
        }
        _ => {
            return Err(failed(
                "Give either `path`, or both `name` and `content_base64`.",
            ));
        }
    };

    let name = crate::saving::safe_name(&name);
    if data.len() > LARGEST_FILE {
        return Err(too_large(&name));
    }
    let carried = mailbox
        .db
        .draft_files_size(&args.draft_id)
        .map_err(failed)?;
    if carried + data.len() as i64 > MOST_FILES_BYTES {
        return Err(failed(
            "That would take this message's files over 25 MB, which is more than most mail systems accept.",
        ));
    }
    let file = mailbox
        .db
        .attach_to_draft(&args.draft_id, &name, content_type(&name), &data)
        .map_err(failed)?
        .ok_or_else(|| failed("There is no such draft, or it is already on its way out."))?;

    Ok(json!({ "file": file_shown(&file) }))
}

fn too_large(name: &str) -> CommandError {
    failed(format!(
        "`{name}` is 3 MB or more, which Brolga cannot attach yet."
    ))
}

async fn remove_file(mailboxes: Arc<Mailboxes>, args: RemoveFile) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;

    Ok(
        json!({ "removed": mailbox.db.detach_from_draft(&args.draft_id, args.file_id).map_err(failed)? }),
    )
}

async fn get_signature(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;
    let (text, on_replies) = signature(&mailbox)?;

    Ok(json!({ "text": text, "on_replies": on_replies }))
}

async fn set_signature(
    mailboxes: Arc<Mailboxes>,
    args: SetSignature,
) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;
    let text = args.text.trim_end().replace("\r\n", "\n");
    if text.chars().count() > LONGEST_SIGNATURE {
        return Err(failed(format!(
            "A signature can be at most {LONGEST_SIGNATURE} characters."
        )));
    }
    mailbox.db.set_setting("signature", &text).map_err(failed)?;
    if let Some(on_replies) = args.on_replies {
        mailbox
            .db
            .set_setting("signature_on_replies", if on_replies { "1" } else { "0" })
            .map_err(failed)?;
    }
    let (text, on_replies) = signature(&mailbox)?;

    Ok(json!({ "text": text, "on_replies": on_replies }))
}

async fn edit(mailboxes: Arc<Mailboxes>, args: Edit) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;
    let change = DraftEdit {
        to_recipients: kept(args.to)?,
        cc_recipients: kept(args.cc)?,
        bcc_recipients: kept(args.bcc)?,
        subject: args.subject,
        body: args.body,
    };
    if !mailbox
        .db
        .edit_draft(&args.draft_id, &change)
        .map_err(failed)?
    {
        return Err(failed(
            match mailbox.db.draft(&args.draft_id).map_err(failed)? {
                Some(_) => {
                    "This message is on its way out and can no longer be changed.".to_owned()
                }
                None => format!("There is no draft `{}`.", args.draft_id),
            },
        ));
    }
    let draft = mailbox.db.draft(&args.draft_id).map_err(failed)?;

    Ok(json!({ "draft": draft.as_ref().map(|d| shown(&mailbox, d)) }))
}

async fn get(mailboxes: Arc<Mailboxes>, args: One) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;
    let draft = mailbox.db.draft(&args.draft_id).map_err(failed)?;

    Ok(json!({ "draft": draft.as_ref().map(|d| shown(&mailbox, d)) }))
}

async fn list(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;
    let drafts = mailbox.db.drafts().map_err(failed)?;

    Ok(json!({ "drafts": drafts.iter().map(|d| shown(&mailbox, d)).collect::<Vec<_>>() }))
}

async fn discard(mailboxes: Arc<Mailboxes>, args: One) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;

    Ok(json!({ "discarded": mailbox.db.discard_draft(&args.draft_id).map_err(failed)? }))
}

async fn send(mailboxes: Arc<Mailboxes>, args: One) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;
    let draft = mailbox
        .db
        .draft(&args.draft_id)
        .map_err(failed)?
        .ok_or_else(|| failed(format!("There is no draft `{}`.", args.draft_id)))?;
    if draft.state == "queued" {
        return Ok(json!({ "queued": true }));
    }
    let nobody = [
        &draft.to_recipients,
        &draft.cc_recipients,
        &draft.bcc_recipients,
    ]
    .iter()
    .all(|list| {
        serde_json::from_str::<Vec<Value>>(list)
            .unwrap_or_default()
            .is_empty()
    });
    if nobody {
        return Err(failed("Add at least one recipient before sending."));
    }

    mailbox.db.queue_draft(&draft.id).map_err(failed)?;
    let sending = mailbox.clone();
    tokio::spawn(async move {
        let _ = sending.outbox.drain().await;
    });

    Ok(json!({ "queued": true }))
}

async fn cancel(mailboxes: Arc<Mailboxes>, args: One) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;

    Ok(json!({ "cancelled": mailbox.db.unqueue_draft(&args.draft_id).map_err(failed)? }))
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

fn account_id() -> Value {
    json!({ "type": "string", "description": "Which account, from accounts.list. May be left out when there is only one." })
}

fn people() -> Value {
    json!({
        "type": "array",
        "maxItems": MOST_RECIPIENTS,
        "items": {
            "type": "object",
            "properties": { "name": { "type": ["string", "null"] }, "address": { "type": "string" } },
            "required": ["address"]
        }
    })
}

fn draft_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "kind": { "enum": ["new", "reply", "reply_all", "forward"] },
            "message_id": { "type": ["string", "null"], "description": "The message answered or forwarded." },
            "to": people(),
            "cc": people(),
            "bcc": people(),
            "subject": { "type": "string" },
            "body": { "type": "string", "description": "The words as written, plain. The original is added under them when it is sent." },
            "state": { "enum": ["editing", "queued", "failed"], "description": "queued: waiting to be sent. failed: Microsoft 365 would not send it; `error` says why." },
            "error": { "type": ["string", "null"] },
            "updated_at": { "type": "integer", "description": "Seconds since 1970." },
            "files": { "type": "array", "items": file_schema(), "description": "The files attached, without their contents." }
        },
        "required": ["id", "kind", "to", "cc", "bcc", "subject", "body", "state"]
    })
}

fn file_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "integer" },
            "name": { "type": "string" },
            "content_type": { "type": "string" },
            "size": { "type": "integer", "description": "In bytes." }
        },
        "required": ["id", "name", "content_type", "size"]
    })
}

fn one_draft() -> Value {
    json!({
        "type": "object",
        "properties": { "draft": { "anyOf": [draft_schema(), { "type": "null" }] } },
        "required": ["draft"]
    })
}

fn one_args() -> Value {
    json!({
        "type": "object",
        "properties": { "account_id": account_id(), "draft_id": { "type": "string" } },
        "required": ["draft_id"],
        "additionalProperties": false
    })
}

pub(crate) fn register(
    registry: &mut Registry,
    mailboxes: Arc<Mailboxes>,
) -> Result<(), RegistryError> {
    registry.register(
        CommandSpec {
            id: "mail.drafts.create".to_owned(),
            title: "Start a message".to_owned(),
            description: "Start a draft: a new message, or with `kind` and `message_id` a reply, a reply to all or a forward. \
                          An answer is addressed and given its RE: or FW: subject unless those are given. The draft is kept on this computer; \
                          nothing is sent until mail.drafts.send. The body is plain words; the original is added under it when it is sent."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "kind": { "enum": ["new", "reply", "reply_all", "forward"], "description": "Default new." },
                    "message_id": { "type": "string", "description": "The message to answer or forward." },
                    "to": people(),
                    "cc": people(),
                    "bcc": people(),
                    "subject": { "type": "string" },
                    "body": { "type": "string" }
                },
                "additionalProperties": false
            }),
            result_schema: one_draft(),
            // The first draft made is `draft-1`, which the examples after this one use.
            examples: vec![
                json!({ "to": [{ "name": "Sam Taylor", "address": "sam@example.test" }], "subject": "Hello", "body": "A first line." }),
                json!({ "kind": "reply_all", "message_id": "message-000008" }),
            ],
        },
        handler(&mailboxes, create),
    )?;

    let signature_result = json!({
        "type": "object",
        "properties": { "text": { "type": "string" }, "on_replies": { "type": "boolean" } },
        "required": ["text", "on_replies"]
    });

    registry.register(
        CommandSpec {
            id: "mail.signature.get".to_owned(),
            title: "Read the signature".to_owned(),
            description: "The account's signature, as plain text, and whether replies and forwards get it as well as new messages. \
                          It is kept on this computer: Microsoft 365 has no public way to read the one set in Outlook."
                .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id() },
                "additionalProperties": false
            }),
            result_schema: signature_result.clone(),
            examples: vec![json!({})],
        },
        handler(&mailboxes, get_signature),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.signature.set".to_owned(),
            title: "Set the signature".to_owned(),
            description: "Set the account's signature, as plain text. A new message starts with it; with `on_replies` true, so do replies and forwards. \
                          An empty text means no signature. Drafts already started are not changed."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "text": { "type": "string", "maxLength": LONGEST_SIGNATURE },
                    "on_replies": { "type": "boolean", "description": "Left out, it stays as it is." }
                },
                "required": ["text"],
                "additionalProperties": false
            }),
            result_schema: signature_result,
            examples: vec![json!({ "text": "Robin Example\nExample Pty Ltd", "on_replies": false })],
        },
        handler(&mailboxes, set_signature),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.drafts.edit".to_owned(),
            title: "Change a draft".to_owned(),
            description: "Change a draft's recipients, subject or body. What is left out stays as it is. \
                          A draft that failed to send becomes an ordinary draft again. One that is waiting to be sent cannot be changed."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "draft_id": { "type": "string" },
                    "to": people(),
                    "cc": people(),
                    "bcc": people(),
                    "subject": { "type": "string" },
                    "body": { "type": "string" }
                },
                "required": ["draft_id"],
                "additionalProperties": false
            }),
            result_schema: one_draft(),
            examples: vec![json!({ "draft_id": "draft-1", "body": "A better first line." })],
        },
        handler(&mailboxes, edit),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.drafts.files.add".to_owned(),
            title: "Attach a file".to_owned(),
            description: "Attach a file to a draft: one on this computer by its `path`, or one given whole as `name` and `content_base64`. \
                          It is copied into the draft, so the original can change or go. A file must be under 3 MB for now, and a message's files 25 MB in all."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "draft_id": { "type": "string" },
                    "path": { "type": "string", "description": "The full path of a file on this computer." },
                    "name": { "type": "string" },
                    "content_base64": { "type": "string" }
                },
                "required": ["draft_id"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "file": file_schema() },
                "required": ["file"]
            }),
            examples: vec![json!({ "draft_id": "draft-1", "name": "notes.txt", "content_base64": "U29tZSBub3Rlcy4=" })],
        },
        handler(&mailboxes, add_file),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.drafts.files.remove".to_owned(),
            title: "Remove an attached file".to_owned(),
            description: "Take a file off a draft, by the id mail.drafts.get lists it under. `removed` is false if there was no such file, or the draft is on its way out."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "draft_id": { "type": "string" },
                    "file_id": { "type": "integer" }
                },
                "required": ["draft_id", "file_id"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "removed": { "type": "boolean" } },
                "required": ["removed"]
            }),
            examples: vec![json!({ "draft_id": "draft-1", "file_id": 999 })],
        },
        handler(&mailboxes, remove_file),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.drafts.get".to_owned(),
            title: "Read a draft".to_owned(),
            description:
                "One draft as it stands on this computer, or null if there is none of that id."
                    .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: one_args(),
            result_schema: one_draft(),
            examples: vec![json!({ "draft_id": "draft-1" })],
        },
        handler(&mailboxes, get),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.drafts.list".to_owned(),
            title: "List drafts".to_owned(),
            description: "The messages being written on this computer and those waiting to be sent, the most recently touched first. \
                          `state` says which each is."
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
                "properties": { "drafts": { "type": "array", "items": draft_schema() } },
                "required": ["drafts"]
            }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, list),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.drafts.discard".to_owned(),
            title: "Discard a draft".to_owned(),
            description: "Throw a draft away. It is gone from this computer and cannot be had back. One waiting to be sent is not discarded: cancel it first."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: one_args(),
            result_schema: json!({
                "type": "object",
                "properties": { "discarded": { "type": "boolean" } },
                "required": ["discarded"]
            }),
            examples: vec![json!({ "draft_id": "no-such-draft" })],
        },
        handler(&mailboxes, discard),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.drafts.send".to_owned(),
            title: "Send a message".to_owned(),
            description: "Send a draft. It is queued and sent in the background, at once if Microsoft 365 can be reached and otherwise when it can, \
                          and it is sent once however many times that takes. It needs at least one recipient. \
                          If Microsoft 365 will not send it, it comes back as a draft with `state` failed and the reason. This cannot be undone once it has gone."
                .to_owned(),
            kind: CommandKind::Action,
            // It reaches other people and cannot be taken back.
            consequential: true,
            mcp_tool: None,
            args_schema: one_args(),
            result_schema: json!({
                "type": "object",
                "properties": { "queued": { "type": "boolean" } },
                "required": ["queued"]
            }),
            examples: vec![json!({ "draft_id": "draft-1" })],
        },
        handler(&mailboxes, send),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.drafts.cancel_send".to_owned(),
            title: "Cancel sending".to_owned(),
            description: "Take a message that is waiting to be sent back to being a draft. This works only while no attempt to send it has been made; \
                          `cancelled` is false if it is too late, or there is no such draft waiting."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: one_args(),
            result_schema: json!({
                "type": "object",
                "properties": { "cancelled": { "type": "boolean" } },
                "required": ["cancelled"]
            }),
            examples: vec![json!({ "draft_id": "draft-1" })],
        },
        handler(&mailboxes, cancel),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_told_from_a_slip_of_the_keyboard() {
        for good in ["sam@example.test", "a.b+c@mail.example.co.uk"] {
            assert!(is_address(good), "{good}");
        }
        for bad in [
            "",
            "sam",
            "sam@",
            "@example.test",
            "sam@example",
            "sam @example.test",
            "sam@example.test, jo@example.test",
            "Sam <sam@example.test>",
            "sam@.test",
            "sam@example.",
        ] {
            assert!(!is_address(bad), "{bad}");
        }
    }

    #[test]
    fn a_subject_is_prefixed_once() {
        assert_eq!(prefixed("RE:", "Lunch"), "RE: Lunch");
        assert_eq!(prefixed("RE:", "re: Lunch"), "re: Lunch");
        assert_eq!(prefixed("FW:", "RE: Lunch"), "FW: RE: Lunch");
    }

    #[test]
    fn a_reply_to_all_goes_to_everyone_but_the_one_replying_and_to_nobody_twice() {
        let source = MessageRow {
            id: "1".to_owned(),
            folder_id: "inbox".to_owned(),
            conversation_id: None,
            subject: "Lunch".to_owned(),
            preview: String::new(),
            from_name: "Sam".to_owned(),
            from_address: "sam@example.test".to_owned(),
            to_recipients: r#"[{"name":"Me","address":"Robin@Example.test"},{"name":"Jo","address":"jo@example.test"},{"name":"Sam","address":"SAM@example.test"}]"#.to_owned(),
            cc_recipients: r#"[{"name":"Al","address":"al@example.test"},{"name":"Jo","address":"jo@example.test"}]"#.to_owned(),
            received_at: String::new(),
            sent_at: None,
            is_read: true,
            is_draft: false,
            has_attachments: false,
            importance: "normal".to_owned(),
            flag_status: "notFlagged".to_owned(),
            categories: "[]".to_owned(),
            inference: "focused".to_owned(),
        };

        let addresses = |raw: String| -> Vec<String> {
            serde_json::from_str::<Vec<Value>>(&raw)
                .unwrap()
                .iter()
                .map(|each| each["address"].as_str().unwrap().to_owned())
                .collect()
        };
        let (to, cc) = answer_to(&source, true, "robin@example.test");
        assert_eq!(addresses(to), ["sam@example.test", "jo@example.test"]);
        assert_eq!(addresses(cc), ["al@example.test"]);

        let (to, cc) = answer_to(&source, false, "robin@example.test");
        assert_eq!(addresses(to), ["sam@example.test"]);
        assert!(addresses(cc).is_empty());
    }
}
