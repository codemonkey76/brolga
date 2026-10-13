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
    format: Option<String>,
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
    format: Option<String>,
    from: Option<String>,
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
    /// A picture to show within the body, not a file beside it.
    inline: Option<bool>,
}

#[derive(Deserialize)]
struct GetFile {
    account_id: Option<String>,
    draft_id: String,
    file_id: i64,
}

/// What kind of picture some bytes are, by how they begin. Only the plain
/// kinds a mail program will show within a message; not by its name, which
/// can say anything.
fn picture_type(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if data.len() > 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[derive(Deserialize)]
struct RemoveFile {
    account_id: Option<String>,
    draft_id: String,
    file_id: i64,
}

/// The most a message's files may come to, and so the largest one file may
/// be. Microsoft 365's own default limit for a whole message is 35 MB, and
/// base64 makes files a third bigger on the way.
const MOST_FILES_BYTES: i64 = 25 * 1024 * 1024;
const LARGEST_FILE: usize = MOST_FILES_BYTES as usize;

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
    json!({
        "id": file.id, "name": file.name, "content_type": file.content_type, "size": file.size,
        "content_id": file.content_id,
    })
}

#[derive(Deserialize)]
struct OpenDraft {
    account_id: Option<String>,
    message_id: String,
}

#[derive(Deserialize)]
struct Suggest {
    account_id: Option<String>,
    typed: String,
    limit: Option<u32>,
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

/// A body as it is kept. A formatted one is cleaned before it is kept, so
/// that what is shown back in the window is already safe to show.
fn body_kept(body: Option<String>, format: Option<&str>) -> Option<String> {
    match (body, format) {
        (Some(body), Some("html")) => {
            Some(brolga_mail::sanitise_html(&body, brolga_mail::RemoteContent::Blocked).html)
        }
        (body, _) => body,
    }
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
        "format": draft.format,
        "from": Some(&draft.from_address).filter(|from| !from.is_empty()),
        "state": draft.state,
        "error": draft.error,
        "updated_at": draft.updated_at,
        "on_server": draft.server_id.is_some(),
        "files": files,
        "web_pictures_left_out": mailbox.db.draft_web_pictures(&draft.id).unwrap_or(0),
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
    if args.kind.as_deref() == Some("forward_as_attachment") {
        return forward_as_attachment(&mailbox, args).await;
    }
    let kind = args.kind.as_deref().unwrap_or("new");
    let mut start = DraftEdit {
        to_recipients: kept(args.to)?,
        cc_recipients: kept(args.cc)?,
        bcc_recipients: kept(args.bcc)?,
        subject: args.subject,
        body: body_kept(args.body, args.format.as_deref()),
        format: args.format,
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
            start.body = Some(if start.format.as_deref() == Some("html") {
                // An empty paragraph to write in, and the signature under it.
                format!("<p><br></p>{}", brolga_mail::text_to_paragraphs(&text))
            } else {
                format!("\n\n{text}")
            });
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
    let file = if args.inline.unwrap_or(false) {
        let kind = picture_type(&data).ok_or_else(|| {
            failed(format!("`{name}` is not a picture that can be shown within a message. PNG, JPEG, GIF and WebP can."))
        })?;
        mailbox.db.attach_inline_to_draft(&args.draft_id, &name, kind, &data)
    } else {
        mailbox.db.attach_to_draft(&args.draft_id, &name, content_type(&name), &data)
    }
    .map_err(failed)?
    .ok_or_else(|| failed("There is no such draft, or it is already on its way out."))?;

    Ok(json!({ "file": file_shown(&file) }))
}

fn too_large(name: &str) -> CommandError {
    failed(format!(
        "`{name}` is over 25 MB, which is more than most mail systems accept. Share it as a link instead."
    ))
}

async fn get_file(mailboxes: Arc<Mailboxes>, args: GetFile) -> Result<Value, CommandError> {
    use base64::Engine;

    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;
    let (file, data) = mailbox
        .db
        .draft_file(&args.draft_id, args.file_id)
        .map_err(failed)?
        .ok_or_else(|| failed("This draft has no such file."))?;
    let mut shown = file_shown(&file);
    shown["content_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(data));

    Ok(json!({ "file": shown }))
}

async fn remove_file(mailboxes: Arc<Mailboxes>, args: RemoveFile) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;

    Ok(
        json!({ "removed": mailbox.db.detach_from_draft(&args.draft_id, args.file_id).map_err(failed)? }),
    )
}

async fn suggest(mailboxes: Arc<Mailboxes>, args: Suggest) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;
    let people = mailbox
        .db
        .suggest_people(&args.typed, args.limit.unwrap_or(6).min(20))
        .map_err(failed)?;
    let limit = args.limit.unwrap_or(6).min(20) as usize;
    let mut people: Vec<Value> = people
        .into_iter()
        .map(|(name, address)| json!({ "name": Some(name).filter(|n| !n.trim().is_empty()), "address": address }))
        .collect();
    // Then personal contacts, and then, while it can be asked, the
    // organisation's directory: each address once.
    let mut seen: std::collections::HashSet<String> = people
        .iter()
        .filter_map(|each| each["address"].as_str().map(str::to_lowercase))
        .collect();
    if people.len() < limit && !args.typed.trim().is_empty() {
        for contact in mailbox.db.contacts(&args.typed, None, 20).map_err(failed)? {
            let Ok(contact) = serde_json::from_str::<Value>(&contact) else {
                continue;
            };
            for email in contact["emails"].as_array().into_iter().flatten() {
                if let Some(address) = email["address"]
                    .as_str()
                    .filter(|address| seen.insert(address.to_lowercase()))
                {
                    people.push(json!({ "name": contact["name"], "address": address }));
                }
            }
        }
    }
    if people.len() < limit {
        let (found, _) = crate::contact_commands::directory(&mailbox, &args.typed).await;
        for person in found {
            if let Some(address) = person["address"]
                .as_str()
                .filter(|address| seen.insert(address.to_lowercase()))
            {
                people.push(json!({ "name": person["name"], "address": address }));
            }
        }
    }
    people.truncate(limit);

    Ok(
        json!({ "people": people, "note": "Names are as they appeared in mail received, and were chosen by whoever sent it." }),
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

/// A new message with another attached to it whole, as a `.eml` file: the
/// way to pass a message on exactly as it came, headers and all.
async fn forward_as_attachment(
    mailbox: &Arc<Mailbox>,
    args: Create,
) -> Result<Value, CommandError> {
    let id = args
        .message_id
        .as_deref()
        .ok_or_else(|| failed("Say which message to forward, with message_id."))?;
    let source = mailbox
        .db
        .message(id)
        .map_err(failed)?
        .ok_or_else(|| failed(format!("There is no message `{id}` on this computer.")))?;
    let mime = mailbox.graph.message_mime(id).await.map_err(|e| {
        failed(format!(
            "The message could not be fetched to attach it. {e}"
        ))
    })?;
    if mime.len() as i64 > MOST_FILES_BYTES {
        return Err(failed(
            "That message is over 25 MB, which is more than most mail systems accept as an attachment.",
        ));
    }

    let mut start = DraftEdit {
        to_recipients: kept(args.to)?,
        cc_recipients: kept(args.cc)?,
        bcc_recipients: kept(args.bcc)?,
        subject: Some(
            args.subject
                .unwrap_or_else(|| prefixed("FW:", &source.subject)),
        ),
        body: body_kept(args.body, args.format.as_deref()),
        format: args.format,
    };
    if start.body.is_none() {
        let (text, _) = signature(mailbox)?;
        if !text.trim().is_empty() {
            start.body = Some(if start.format.as_deref() == Some("html") {
                format!("<p><br></p>{}", brolga_mail::text_to_paragraphs(&text))
            } else {
                format!("\n\n{text}")
            });
        }
    }
    let draft = mailbox
        .db
        .create_draft("new", None, &start)
        .map_err(failed)?;

    let subject = if source.subject.trim().is_empty() {
        "Message"
    } else {
        source.subject.trim()
    };
    let name = crate::saving::safe_name(&format!("{subject}.eml"));
    mailbox
        .db
        .attach_to_draft(&draft.id, &name, "message/rfc822", &mime)
        .map_err(failed)?;

    Ok(json!({ "draft": shown(mailbox, &draft) }))
}

async fn edit(mailboxes: Arc<Mailboxes>, args: Edit) -> Result<Value, CommandError> {
    let (mailbox, own_address) = open(&mailboxes, args.account_id.as_deref()).await?;
    // A body is cleaned as the format it is being saved in: the one given
    // with it, or else the draft's own.
    let format = match (&args.format, &args.body) {
        (Some(format), _) => Some(format.clone()),
        (None, Some(_)) => mailbox
            .db
            .draft(&args.draft_id)
            .map_err(failed)?
            .map(|d| d.format),
        (None, None) => None,
    };
    // A change of format alone still has the words there already to clean
    // as the new one: plain text is not HTML until it has been made safe.
    let body = match (args.body, &args.format) {
        (None, Some(_)) => mailbox
            .db
            .draft(&args.draft_id)
            .map_err(failed)?
            .map(|d| d.body),
        (body, _) => body,
    };
    // Looked at before anything is changed, so that a change that is
    // turned down is turned down whole.
    if let Some(from) = args.from.as_deref().map(str::trim)
        && !from.is_empty()
        && (!from.contains('@') || from.contains(char::is_whitespace))
    {
        return Err(failed(format!("`{from}` is not an email address.")));
    }
    let change = DraftEdit {
        to_recipients: kept(args.to)?,
        cc_recipients: kept(args.cc)?,
        bcc_recipients: kept(args.bcc)?,
        subject: args.subject,
        body: body_kept(body, format.as_deref()),
        format: args.format,
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
    if let Some(from) = &args.from {
        let from = from.trim();
        // Put back to the mailbox's own name, that name is said outright:
        // the server's copy may still carry the other.
        let from = if from.is_empty() {
            own_address.as_str()
        } else {
            from
        };
        mailbox
            .db
            .set_draft_from(&args.draft_id, from)
            .map_err(failed)?;
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
    let server_id = mailbox
        .db
        .draft(&args.draft_id)
        .map_err(failed)?
        .and_then(|draft| draft.server_id);
    let discarded = mailbox.db.discard_draft(&args.draft_id).map_err(failed)?;

    // Its copy on the server goes too, to Deleted Items as anything
    // deleted does. Through the outbox, so it happens offline as well.
    // Where the copy has not been synced down yet there is no message
    // here to move, and the copy is noted for the outbox by its id alone.
    if let (true, Some(server_id)) = (discarded, server_id) {
        let queued = match mailbox.db.folder_id("deleteditems").map_err(failed)? {
            Some(deleted) => {
                let action = mailbox.db.new_action().map_err(failed)?;
                mailbox
                    .db
                    .queue_edit(
                        &action,
                        std::slice::from_ref(&server_id),
                        &brolga_store::MessageEdit::Move(deleted),
                    )
                    .map_err(failed)?
            }
            None => 0,
        };
        if queued == 0 {
            mailbox.db.discard_on_server(&server_id).map_err(failed)?;
        }
        let sending = mailbox.clone();
        tokio::spawn(async move {
            let _ = sending.outbox.drain().await;
        });
    }

    Ok(json!({ "discarded": discarded }))
}

/// The name a body's picture refers to an attached one by, where it is one
/// that can be looked for in the body as it is written, letter for letter:
/// `cid:` and plain characters, none of which HTML writes another way.
fn plain_picture_name(address: &str) -> Option<&str> {
    address.strip_prefix("cid:").filter(|name| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_graphic() && !matches!(c, '&' | '<' | '>' | '"' | '\''))
    })
}

/// Make the draft's own the pictures within a body written in another mail
/// program, which the server's copy carries as inline attachments ("Get
/// attachment", SOURCES.md): each is kept here under the name the body
/// calls it by and its id on that copy, so it is shown as it is written
/// around, and goes with the message as it came.
///
/// The limits are those for the pictures of mail that is read. One left
/// out, or that cannot be fetched now, stays on the server's copy and in
/// the body as it was; it is only not shown.
async fn take_up_pictures(mailbox: &Mailbox, server_id: &str, draft: &Draft) {
    use crate::mail_commands::{MAX_INLINE_BYTES, MAX_INLINE_IMAGES};
    use base64::Engine;

    let sources = brolga_mail::image_sources(&draft.body);
    let mut wanted: Vec<&str> = sources
        .iter()
        .filter_map(|source| plain_picture_name(&source.address))
        .collect();
    if wanted.is_empty() {
        return;
    }
    let Ok(attached) = mailbox.graph.attachments(server_id).await else {
        return;
    };

    for each in attached
        .iter()
        .filter(|a| a.is_inline == Some(true) && a.kind() == "file")
        .filter(|a| a.size.is_some_and(|size| size <= MAX_INLINE_BYTES))
        .take(MAX_INLINE_IMAGES)
    {
        let Ok(whole) = mailbox.graph.file_attachment(server_id, &each.id).await else {
            continue;
        };
        let (Some(content_id), Some(content)) = (whole.content_id, whole.content_bytes) else {
            continue;
        };
        let content_id = content_id.trim_matches(['<', '>']);
        // By the body's own spelling of the name, which is what is looked
        // for in the body when it is asked whether the picture is still in it.
        let Some(at) = wanted
            .iter()
            .position(|name| name.eq_ignore_ascii_case(content_id))
        else {
            continue;
        };
        let Ok(data) = base64::engine::general_purpose::STANDARD.decode(content.trim()) else {
            continue;
        };
        // By how it begins, not by what it is said to be.
        let Some(kind) = picture_type(&data) else {
            continue;
        };
        let name = crate::saving::safe_name(each.name.as_deref().unwrap_or("picture"));
        if mailbox
            .db
            .adopt_draft_picture(&draft.id, &name, kind, &data, wanted[at], &each.id)
            .is_ok()
        {
            wanted.remove(at);
        }
    }
}

/// Carry on with a draft that is in the mailbox's Drafts folder: the one
/// already here for it, or else a copy taken from the server now.
async fn open_draft(mailboxes: Arc<Mailboxes>, args: OpenDraft) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;
    if let Some(here) = mailbox
        .db
        .draft_for_message(&args.message_id)
        .map_err(failed)?
    {
        return Ok(json!({ "draft": shown(&mailbox, &here) }));
    }

    let server = mailbox.graph.draft(&args.message_id).await.map_err(|e| {
        failed(format!(
            "This draft is not on this computer yet, and could not be fetched now. {e}"
        ))
    })?;
    if server.is_draft == Some(false) {
        return Err(failed("That message is not a draft."));
    }
    let people = |list: Option<Vec<brolga_graph::Recipient>>| {
        let list: Vec<Value> = list
            .unwrap_or_default()
            .into_iter()
            .filter_map(|each| {
                let address = each.email_address.address?;
                Some(json!({ "name": each.email_address.name, "address": address }))
            })
            .collect();
        Value::Array(list).to_string()
    };
    // What was written elsewhere is cleaned like any other mail before it
    // is kept, and is formatted whatever it was written as.
    let mut web_pictures = 0;
    let body = server.body.map(|body| {
        if body.content_type.eq_ignore_ascii_case("html") {
            let clean =
                brolga_mail::sanitise_html(&body.content, brolga_mail::RemoteContent::Blocked);
            web_pictures = clean.blocked_remote;
            clean.html
        } else {
            brolga_mail::text_to_paragraphs(&body.content)
        }
    });
    let content = DraftEdit {
        to_recipients: Some(people(server.to_recipients)),
        cc_recipients: Some(people(server.cc_recipients)),
        bcc_recipients: Some(people(server.bcc_recipients)),
        subject: server.subject,
        body,
        format: Some("html".to_owned()),
    };
    let draft = mailbox
        .db
        .adopt_draft(&args.message_id, &content)
        .map_err(failed)?;
    if web_pictures > 0 {
        mailbox
            .db
            .set_draft_web_pictures(&draft.id, web_pictures)
            .map_err(failed)?;
    }
    take_up_pictures(&mailbox, &args.message_id, &draft).await;

    Ok(json!({ "draft": shown(&mailbox, &draft) }))
}

/// The pictures within a draft that are on the server's copy of it and
/// are not files of the draft here: those of a draft begun in another mail
/// program, such as the pictures in a message it quotes. They are sent
/// with it either way; this is for showing them while it is written.
async fn pictures(mailboxes: Arc<Mailboxes>, args: One) -> Result<Value, CommandError> {
    let (mailbox, _) = open(&mailboxes, args.account_id.as_deref()).await?;
    let draft = mailbox
        .db
        .draft(&args.draft_id)
        .map_err(failed)?
        .ok_or_else(|| failed(format!("There is no draft `{}`.", args.draft_id)))?;
    let Some(server_id) = &draft.server_id else {
        return Ok(json!({ "pictures": [] }));
    };
    let pictures: Vec<Value> = crate::mail_commands::inline_images(&mailbox, server_id)
        .await
        .iter()
        .map(|each| {
            json!({
                "content_id": each.content_id,
                "content_type": each.content_type,
                "content_base64": each.data,
            })
        })
        .collect();

    Ok(json!({ "pictures": pictures }))
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
            "body": { "type": "string", "description": "The words as written: plain, or HTML, as `format` says. The original is added under them when it is sent." },
            "format": { "enum": ["text", "html"] },
            "state": { "enum": ["editing", "queued", "failed"], "description": "queued: waiting to be sent. failed: Microsoft 365 would not send it; `error` says why." },
            "error": { "type": ["string", "null"] },
            "updated_at": { "type": "integer", "description": "Seconds since 1970." },
            "on_server": { "type": "boolean", "description": "It has a copy in the mailbox's Drafts folder, so other mail programs show it." },
            "files": { "type": "array", "items": file_schema(), "description": "The files attached, without their contents." },
            "web_pictures_left_out": { "type": "integer", "description": "On a draft written in another mail program: how many pictures kept on the web its body had. They are not in `body`, and are gone from the draft for good once it is changed here." }
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
            "size": { "type": "integer", "description": "In bytes." },
            "content_id": { "type": ["string", "null"], "description": "For a picture shown within the body: what the body calls it, as <img src=\"cid:…\">." },
            "content_base64": { "type": "string", "description": "The contents, where they were asked for." }
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
                          nothing is sent until mail.drafts.send. The body is plain words unless `format` is html; the original is added under it when it is sent."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "kind": {
                        "enum": ["new", "reply", "reply_all", "forward", "forward_as_attachment"],
                        "description": "Default new. forward_as_attachment starts a new message with the one named attached whole, as a .eml file; it needs the network."
                    },
                    "message_id": { "type": "string", "description": "The message to answer or forward." },
                    "to": people(),
                    "cc": people(),
                    "bcc": people(),
                    "subject": { "type": "string" },
                    "body": { "type": "string" },
                    "format": { "enum": ["text", "html"], "description": "Default text. As html, the body is cleaned before it is kept: plain formatting, lists and links stay; scripts, styles and remote pictures do not." }
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

    registry.register(
        CommandSpec {
            id: "mail.people.suggest".to_owned(),
            title: "Suggest recipients".to_owned(),
            description: "People to offer while an address is being typed: those in mail on this computer whose name or address has a word \
                          starting with `typed`, the ones written to most first. Then your personal contacts, and, while Microsoft 365 can be reached, your organisation's directory."
                .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id(),
                    "typed": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20, "description": "Default 6." }
                },
                "required": ["typed"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "people": people(), "note": { "type": "string" } },
                "required": ["people", "note"]
            }),
            examples: vec![json!({ "typed": "sam" })],
        },
        handler(&mailboxes, suggest),
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
            consequential: true,
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
                    "body": { "type": "string" },
                    "format": { "enum": ["text", "html"] },
                    "from": { "type": "string", "description": "The address it is to go out under: a mailbox you may send as or on behalf of. Empty for your own. Microsoft 365 refuses the send if you may not." }
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
            id: "mail.drafts.open".to_owned(),
            title: "Carry on with a draft".to_owned(),
            description: "Take up a draft that is in the mailbox's Drafts folder, by its message id, to change or send it: the draft already here for it, \
                          or a copy fetched now if it was written in another mail program. Pictures within its body that are attached to it come with it, as files with a `content_id`. \
                          Formatting this client does not keep, such as fonts and colours, and pictures kept on the web (`web_pictures_left_out` counts them) \
                          are lost from a draft written elsewhere once it is changed here."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id(), "message_id": { "type": "string" } },
                "required": ["message_id"],
                "additionalProperties": false
            }),
            result_schema: one_draft(),
            // The draft the command tests leave in the fake mailbox's Drafts folder.
            examples: vec![json!({ "message_id": "message-000009" })],
        },
        handler(&mailboxes, open_draft),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.drafts.files.add".to_owned(),
            title: "Attach a file".to_owned(),
            description: "Attach a file to a draft: one on this computer by its `path`, or one given whole as `name` and `content_base64`. \
                          It is copied into the draft, so the original can change or go. A message's files may come to 25 MB in all."
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
                    "content_base64": { "type": "string" },
                    "inline": { "type": "boolean", "description": "A picture to show within the body: PNG, JPEG, GIF or WebP. It comes back with a `content_id`; put <img src=\"cid:CONTENT_ID\"> in a formatted body where it should appear." }
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
            id: "mail.drafts.files.get".to_owned(),
            title: "Read an attached file".to_owned(),
            description: "One file of a draft with its contents in base64, by the id mail.drafts.get lists it under. \
                          The window uses it to show a draft's pictures; the draft and its files are the user's own."
                .to_owned(),
            kind: CommandKind::Query,
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
                "properties": { "file": file_schema() },
                "required": ["file"]
            }),
            // The file the example of mail.drafts.files.add attached.
            examples: vec![json!({ "draft_id": "draft-1", "file_id": 1 })],
        },
        handler(&mailboxes, get_file),
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
            id: "mail.drafts.pictures".to_owned(),
            title: "Read a draft's pictures from the mailbox".to_owned(),
            description: "The pictures shown within a draft's body that are kept with the mailbox's copy of it and not on this computer: \
                          those of a draft begun in another mail program, such as the pictures in a message it quotes. Each is what the body calls \
                          `cid:CONTENT_ID`. They are sent with the draft whether or not they are read here. None for a draft that is only on this computer, \
                          or when Microsoft 365 is out of reach. PNG, JPEG, GIF and WebP only."
                .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: one_args(),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "pictures": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "content_id": { "type": "string" },
                                "content_type": { "type": "string" },
                                "content_base64": { "type": "string" }
                            },
                            "required": ["content_id", "content_type", "content_base64"]
                        }
                    }
                },
                "required": ["pictures"]
            }),
            examples: vec![json!({ "draft_id": "draft-1" })],
        },
        handler(&mailboxes, pictures),
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
            flag_due: String::new(),
            is_meeting: false,
            last_action: String::new(),
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
