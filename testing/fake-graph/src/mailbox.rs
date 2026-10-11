//! A mailbox for the fake Graph server: folders and messages, and the delta
//! queries over them, behaving as "Use delta query to track changes" and the
//! mailFolder and message delta pages document (SOURCES.md).
//!
//! Every change is given a sequence number and logged. A delta token is a
//! sequence number: a later request is answered with what was logged since.

use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

#[derive(Clone)]
struct Folder {
    name: String,
    parent: String,
    hidden: bool,
}

/// A message to put in the fake mailbox. The people are made up.
#[derive(Debug, Clone)]
pub struct FakeMessage {
    pub subject: String,
    pub from_name: String,
    pub from_address: String,
    /// ISO 8601, UTC.
    pub received: String,
    pub is_read: bool,
    pub has_attachments: bool,
    pub body_html: String,
    /// The conversation it belongs to. Left out, it is one of its own.
    pub conversation: Option<String>,
    /// `notFlagged`, `flagged` or `complete`.
    pub flag: String,
    /// The day a flag is due, `YYYY-MM-DD`.
    pub flag_due: Option<String>,
    pub is_draft: bool,
    pub categories: Vec<String>,
    /// What it says of a meeting, if it is about one.
    pub meeting: Option<FakeMeeting>,
    /// `(name, address)`. Left empty, the message is to the mailbox's owner.
    pub to: Vec<(String, String)>,
    pub cc: Vec<(String, String)>,
    pub bcc: Vec<(String, String)>,
    pub attachments: Vec<FakeAttachment>,
}

/// A message about a meeting.
#[derive(Clone, Debug)]
pub struct FakeMeeting {
    /// `meetingRequest`, `meetingCancelled`, `meetingAccepted`,
    /// `meetingTenativelyAccepted` or `meetingDeclined`.
    pub kind: String,
    /// UTC: `2026-10-20T03:00:00Z`.
    pub start: String,
    pub end: String,
    pub location: String,
    /// The event in the calendar it is about. An answer has none of its own.
    pub event_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FakeAttachment {
    pub name: String,
    pub content_type: String,
    pub content: Vec<u8>,
    pub inline: bool,
    /// `file`, `item` or `reference`.
    pub kind: &'static str,
    /// What the body calls it by, for a picture shown within it.
    pub content_id: Option<String>,
}

impl FakeMessage {
    pub fn new(subject: &str, from_name: &str, from_address: &str, received: &str) -> Self {
        Self {
            subject: subject.to_owned(),
            from_name: from_name.to_owned(),
            from_address: from_address.to_owned(),
            received: received.to_owned(),
            is_read: false,
            has_attachments: false,
            body_html: format!("<html><body><p>{subject}</p></body></html>"),
            conversation: None,
            flag: "notFlagged".to_owned(),
            flag_due: None,
            is_draft: false,
            categories: Vec::new(),
            meeting: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            attachments: Vec::new(),
        }
    }

    pub fn body(mut self, html: &str) -> Self {
        self.body_html = html.to_owned();
        self
    }

    /// Attach a file.
    pub fn attach(mut self, name: &str, content_type: &str, content: &[u8]) -> Self {
        self.attachments.push(FakeAttachment {
            name: name.to_owned(),
            content_type: content_type.to_owned(),
            content: content.to_vec(),
            inline: false,
            kind: "file",
            content_id: None,
        });
        self.has_attachments = true;
        self
    }

    /// Attach a picture the body shows as `<img src="cid:{content_id}">`.
    /// As in real mail, it does not make the message one "with attachments".
    pub fn attach_inline(
        mut self,
        name: &str,
        content_type: &str,
        content_id: &str,
        content: &[u8],
    ) -> Self {
        self.attachments.push(FakeAttachment {
            name: name.to_owned(),
            content_type: content_type.to_owned(),
            content: content.to_vec(),
            inline: true,
            kind: "file",
            content_id: Some(content_id.to_owned()),
        });
        self
    }

    /// Attach a link to a file kept elsewhere, which has no contents.
    pub fn attach_link(mut self, name: &str) -> Self {
        self.attachments.push(FakeAttachment {
            name: name.to_owned(),
            content_type: "application/octet-stream".to_owned(),
            content: Vec::new(),
            inline: false,
            kind: "reference",
            content_id: None,
        });
        self.has_attachments = true;
        self
    }

    /// Make it a message about a meeting.
    pub fn about_meeting(
        mut self,
        kind: &str,
        start: &str,
        end: &str,
        location: &str,
        event_id: Option<&str>,
    ) -> Self {
        self.meeting = Some(FakeMeeting {
            kind: kind.to_owned(),
            start: start.to_owned(),
            end: end.to_owned(),
            location: location.to_owned(),
            event_id: event_id.map(str::to_owned),
        });
        self
    }

    pub fn conversation(mut self, id: &str) -> Self {
        self.conversation = Some(id.to_owned());
        self
    }

    pub fn read(mut self) -> Self {
        self.is_read = true;
        self
    }
}

#[derive(Clone)]
struct Stored {
    folder: String,
    message: FakeMessage,
}

struct Change {
    seq: u64,
    id: String,
    removed: bool,
}

pub struct Mailbox {
    seq: u64,
    /// Raised to make every delta token handed out so far stale.
    epoch: u64,
    /// Answer every delta request, even a first one, as stale.
    pub(crate) never_settles: bool,
    next_id: u64,
    root: String,
    folders: BTreeMap<String, Folder>,
    well_known: HashMap<&'static str, String>,
    folder_log: Vec<Change>,
    messages: HashMap<String, Stored>,
    message_logs: HashMap<String, Vec<Change>>,
}

/// What a delta request is answered with, before it is turned into HTTP.
pub enum Answer {
    Page(Value),
    /// The token is from before the tokens were expired.
    Gone,
    NotFound,
}

impl Default for Mailbox {
    fn default() -> Self {
        let mut mailbox = Self {
            seq: 0,
            epoch: 1,
            never_settles: false,
            next_id: 0,
            root: "root".to_owned(),
            folders: BTreeMap::new(),
            well_known: HashMap::new(),
            folder_log: Vec::new(),
            messages: HashMap::new(),
            message_logs: HashMap::new(),
        };

        for (known, name) in [
            ("inbox", "Inbox"),
            ("drafts", "Drafts"),
            ("sentitems", "Sent Items"),
            ("deleteditems", "Deleted Items"),
            ("archive", "Archive"),
            ("junkemail", "Junk Email"),
            ("outbox", "Outbox"),
        ] {
            let id = mailbox.add_folder(name, None);
            mailbox.well_known.insert(known, id);
        }

        mailbox
    }
}

impl Mailbox {
    fn tick(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn new_id(&mut self, kind: &str) -> String {
        self.next_id += 1;
        format!("{kind}-{:06}", self.next_id)
    }

    pub fn folder_id(&self, name_or_id: &str) -> Option<String> {
        self.well_known
            .get(name_or_id.to_ascii_lowercase().as_str())
            .cloned()
            .or_else(|| {
                self.folders
                    .contains_key(name_or_id)
                    .then(|| name_or_id.to_owned())
            })
    }

    pub fn add_folder(&mut self, name: &str, parent: Option<&str>) -> String {
        let id = self.new_id("folder");
        let parent = parent
            .map(str::to_owned)
            .unwrap_or_else(|| self.root.clone());
        self.folders.insert(
            id.clone(),
            Folder {
                name: name.to_owned(),
                parent,
                hidden: false,
            },
        );
        let seq = self.tick();
        self.folder_log.push(Change {
            seq,
            id: id.clone(),
            removed: false,
        });

        id
    }

    pub fn rename_folder(&mut self, id: &str, name: &str) {
        if let Some(folder) = self.folders.get_mut(id) {
            folder.name = name.to_owned();
            let seq = self.tick();
            self.folder_log.push(Change {
                seq,
                id: id.to_owned(),
                removed: false,
            });
        }
    }

    pub fn remove_folder(&mut self, id: &str) {
        if self.folders.remove(id).is_some() {
            self.messages.retain(|_, stored| stored.folder != id);
            self.message_logs.remove(id);
            let seq = self.tick();
            self.folder_log.push(Change {
                seq,
                id: id.to_owned(),
                removed: true,
            });
        }
    }

    fn log_message(&mut self, folder: &str, id: &str, removed: bool) {
        let seq = self.tick();
        self.message_logs
            .entry(folder.to_owned())
            .or_default()
            .push(Change {
                seq,
                id: id.to_owned(),
                removed,
            });
        // A folder's counts change with its messages.
        self.folder_log.push(Change {
            seq,
            id: folder.to_owned(),
            removed: false,
        });
    }

    pub fn add_message(&mut self, folder: &str, message: FakeMessage) -> String {
        let id = self.new_id("message");
        self.messages.insert(
            id.clone(),
            Stored {
                folder: folder.to_owned(),
                message,
            },
        );
        self.log_message(folder, &id, false);

        id
    }

    pub fn set_flag(&mut self, id: &str, flag: &str) {
        if let Some(stored) = self.messages.get_mut(id) {
            stored.message.flag = flag.to_owned();
            let folder = stored.folder.clone();
            self.log_message(&folder, id, false);
        }
    }

    /// A message's flag, as the server has it.
    pub fn flag(&self, id: &str) -> Option<String> {
        self.messages
            .get(id)
            .map(|stored| stored.message.flag.clone())
    }

    pub fn set_read(&mut self, id: &str, is_read: bool) {
        if let Some(stored) = self.messages.get_mut(id) {
            stored.message.is_read = is_read;
            let folder = stored.folder.clone();
            self.log_message(&folder, id, false);
        }
    }

    pub fn set_subject(&mut self, id: &str, subject: &str) {
        if let Some(stored) = self.messages.get_mut(id) {
            stored.message.subject = subject.to_owned();
            let folder = stored.folder.clone();
            self.log_message(&folder, id, false);
        }
    }

    /// With immutable ids, a moved message keeps its id: it is removed from
    /// one folder's collection and appears in the other's.
    pub fn move_message(&mut self, id: &str, to: &str) {
        let Some(stored) = self.messages.get_mut(id) else {
            return;
        };
        let from = std::mem::replace(&mut stored.folder, to.to_owned());
        self.log_message(&from, id, true);
        self.log_message(to, id, false);
    }

    /// A copy of a message as the server holds it.
    pub fn stored(&self, id: &str) -> Option<FakeMessage> {
        self.messages.get(id).map(|stored| stored.message.clone())
    }

    /// Change a draft in place, and note that it changed.
    pub fn edit(&mut self, id: &str, change: impl FnOnce(&mut FakeMessage)) {
        if let Some(stored) = self.messages.get_mut(id) {
            change(&mut stored.message);
            let folder = stored.folder.clone();
            self.log_message(&folder, id, false);
        }
    }

    pub fn delete_message(&mut self, id: &str) {
        if let Some(stored) = self.messages.remove(id) {
            self.log_message(&stored.folder, id, true);
        }
    }

    pub fn expire_delta_tokens(&mut self) {
        self.epoch += 1;
    }

    /// Every message as it truly is: id to (folder, subject, read).
    pub fn messages(&self) -> BTreeMap<String, (String, String, bool)> {
        self.messages
            .iter()
            .map(|(id, s)| {
                (
                    id.clone(),
                    (
                        s.folder.clone(),
                        s.message.subject.clone(),
                        s.message.is_read,
                    ),
                )
            })
            .collect()
    }

    /// Every folder as it truly is: id to (name, parent).
    pub fn folders(&self) -> BTreeMap<String, (String, String)> {
        self.folders
            .iter()
            .map(|(id, f)| (id.clone(), (f.name.clone(), f.parent.clone())))
            .collect()
    }

    fn folder_json(&self, id: &str) -> Option<Value> {
        let folder = self.folders.get(id)?;
        let mine = || self.messages.values().filter(|s| s.folder == id);

        Some(json!({
            "id": id,
            "displayName": folder.name,
            "parentFolderId": folder.parent,
            "childFolderCount": self.folders.values().filter(|f| f.parent == id).count(),
            "unreadItemCount": mine().filter(|s| !s.message.is_read).count(),
            "totalItemCount": mine().count(),
            "isHidden": folder.hidden,
        }))
    }

    pub fn folder(&self, name_or_id: &str) -> Option<Value> {
        self.folder_json(&self.folder_id(name_or_id)?)
    }

    fn message_json(&self, id: &str, with_body: bool) -> Option<Value> {
        let stored = self.messages.get(id)?;
        let m = &stored.message;
        let preview: String = strip_tags(&m.body_html).chars().take(255).collect();

        let mut value = json!({
            "id": id,
            "parentFolderId": stored.folder,
            "conversationId": m.conversation.clone().unwrap_or_else(|| format!("conversation-{id}")),
            "subject": m.subject,
            "bodyPreview": preview,
            "from": { "emailAddress": { "name": m.from_name, "address": m.from_address } },
            "toRecipients": if m.to.is_empty() && !m.is_draft {
                json!([{ "emailAddress": { "name": "Robin Example", "address": "robin@example.test" } }])
            } else {
                recipients(&m.to)
            },
            "ccRecipients": recipients(&m.cc),
            "bccRecipients": recipients(&m.bcc),
            "receivedDateTime": m.received,
            "sentDateTime": m.received,
            "isRead": m.is_read,
            "isDraft": m.is_draft,
            "hasAttachments": m.has_attachments,
            "importance": "normal",
            "flag": match &m.flag_due {
                Some(day) => json!({
                    "flagStatus": m.flag,
                    "startDateTime": { "dateTime": format!("{day}T00:00:00.0000000"), "timeZone": "UTC" },
                    "dueDateTime": { "dateTime": format!("{day}T00:00:00.0000000"), "timeZone": "UTC" }
                }),
                None => json!({ "flagStatus": m.flag }),
            },
            "categories": m.categories,
            "inferenceClassification": "focused",
        });
        if with_body {
            value["body"] = json!({ "contentType": "html", "content": m.body_html });
        }
        // "eventMessage resource type": a message about a meeting says what
        // kind it is, in a list as well as on its own.
        if let Some(meeting) = &m.meeting {
            let moment = |at: &str| json!({ "dateTime": format!("{}.0000000", at.trim_end_matches('Z')), "timeZone": "UTC" });
            value["@odata.type"] = json!(if meeting.kind == "meetingRequest"
                || meeting.kind == "meetingCancelled"
            {
                "#microsoft.graph.eventMessageRequest"
            } else {
                "#microsoft.graph.eventMessageResponse"
            });
            if with_body {
                value["meetingMessageType"] = json!(meeting.kind);
                value["startDateTime"] = moment(&meeting.start);
                value["endDateTime"] = moment(&meeting.end);
                value["location"] = json!({ "displayName": meeting.location });
                value["isAllDay"] = json!(false);
            }
        }

        Some(value)
    }

    pub fn message(&self, id: &str) -> Option<Value> {
        self.message_json(id, true)
    }

    /// The id of the event a message about a meeting is about.
    pub fn meeting_event(&self, id: &str) -> Option<String> {
        self.messages
            .get(id)?
            .message
            .meeting
            .as_ref()?
            .event_id
            .clone()
    }

    /// A message's attachments as Graph lists them. The contents are given
    /// only when they were not left out by `$select`, as Graph does.
    pub fn attachments(&self, message: &str, with_content: bool) -> Option<Value> {
        use base64::Engine;

        let stored = self.messages.get(message)?;
        let listed: Vec<Value> = stored
            .message
            .attachments
            .iter()
            .enumerate()
            .map(|(at, a)| {
                let mut value = json!({
                    "@odata.type": format!("#microsoft.graph.{}Attachment", a.kind),
                    "id": format!("attachment-{at}"),
                    "name": a.name,
                    "contentType": a.content_type,
                    "size": a.content.len(),
                    "isInline": a.inline,
                });
                if with_content && a.kind == "file" {
                    value["contentId"] = json!(a.content_id);
                    value["contentBytes"] =
                        json!(base64::engine::general_purpose::STANDARD.encode(&a.content));
                }
                value
            })
            .collect();

        Some(json!({ "value": listed }))
    }

    /// One attachment as Graph gives it when asked for by id: whole.
    pub fn attachment_json(&self, message: &str, attachment: &str) -> Option<Value> {
        let at: usize = attachment.strip_prefix("attachment-")?.parse().ok()?;

        self.attachments(message, true)?["value"].get(at).cloned()
    }

    /// One attachment: its kind, content type and contents.
    pub fn attachment(&self, message: &str, attachment: &str) -> Option<FakeAttachment> {
        let at: usize = attachment.strip_prefix("attachment-")?.parse().ok()?;

        self.messages
            .get(message)?
            .message
            .attachments
            .get(at)
            .cloned()
    }

    pub fn folders_delta(&self, token: Option<&str>, page_size: usize, link: &str) -> Answer {
        let snapshot = || {
            self.folders
                .keys()
                .map(|id| (String::new(), id.clone()))
                .collect::<Vec<_>>()
        };

        self.delta(token, page_size, link, &self.folder_log, snapshot, |id| {
            self.folder_json(id)
        })
    }

    pub fn messages_delta(
        &self,
        folder: &str,
        token: Option<&str>,
        page_size: usize,
        link: &str,
    ) -> Answer {
        let Some(folder) = self.folder_id(folder) else {
            return Answer::NotFound;
        };
        let empty = Vec::new();
        let log = self.message_logs.get(&folder).unwrap_or(&empty);
        // Newest first, as `$orderby=receivedDateTime desc` asks.
        let snapshot = || {
            let mut mine: Vec<(&String, &Stored)> = self
                .messages
                .iter()
                .filter(|(_, s)| s.folder == folder)
                .collect();
            mine.sort_by(|a, b| {
                b.1.message
                    .received
                    .cmp(&a.1.message.received)
                    .then(a.0.cmp(b.0))
            });
            mine.into_iter()
                .map(|(id, s)| (s.message.received.clone(), id.clone()))
                .collect()
        };

        self.delta(token, page_size, link, log, snapshot, |id| {
            // A message logged in this folder and since moved out of it is not this folder's to report.
            self.messages
                .get(id)
                .filter(|s| s.folder == folder)
                .and_then(|_| self.message_json(id, false))
        })
    }

    /// One page of a delta query. Tokens are:
    ///
    /// - `s.{epoch}.{start}.{received}.{id}`: paging through the state as it
    ///   stands, in a round that began at sequence `start`, having reached
    ///   the item with that date and id. Paging by the last item reached,
    ///   not by a count, means an item deleted mid-round cannot make the
    ///   next page skip one;
    /// - `d.{epoch}.{since}`: the round is done; ask for changes after `since`;
    /// - `c.{epoch}.{since}.{upto}.{offset}`: paging through those changes.
    fn delta(
        &self,
        token: Option<&str>,
        page_size: usize,
        link: &str,
        log: &[Change],
        // Each item as (date, id), in the order they are paged: newest first, then by id.
        snapshot: impl Fn() -> Vec<(String, String)>,
        render: impl Fn(&str) -> Option<Value>,
    ) -> Answer {
        let page_size = page_size.max(1);
        let parts: Vec<&str> = token.map(|t| t.split('.').collect()).unwrap_or_default();
        let number = |at: usize| parts.get(at).and_then(|p| p.parse::<u64>().ok());

        if self.never_settles || (token.is_some() && number(1) != Some(self.epoch)) {
            return Answer::Gone;
        }

        let (entries, next): (Vec<Value>, String) = match parts.first().copied() {
            None | Some("s") => {
                let start = number(2).unwrap_or(self.seq);
                let reached = parts.get(3).zip(parts.get(4));
                let after = |item: &(String, String)| match reached {
                    None => true,
                    Some((date, id)) => {
                        item.0.as_str() < *date || (item.0 == *date && item.1.as_str() > *id)
                    }
                };
                let left: Vec<(String, String)> = snapshot().into_iter().filter(after).collect();
                let page: Vec<Value> = left
                    .iter()
                    .take(page_size)
                    .filter_map(|(_, id)| render(id))
                    .collect();

                match left
                    .get(page_size.saturating_sub(1))
                    .filter(|_| left.len() > page_size)
                {
                    Some((date, id)) => (
                        page,
                        format!("{link}?$skiptoken=s.{}.{start}.{date}.{id}", self.epoch),
                    ),
                    None => (page, format!("{link}?$deltatoken=d.{}.{start}", self.epoch)),
                }
            }
            Some(kind @ ("d" | "c")) => {
                let Some(since) = number(2) else {
                    return Answer::Gone;
                };
                let (upto, offset) = match kind {
                    "c" => (
                        number(3).unwrap_or(self.seq),
                        number(4).unwrap_or(0) as usize,
                    ),
                    _ => (self.seq, 0),
                };

                // Each item once, as its latest change in the window.
                let mut latest: Vec<&Change> = Vec::new();
                for change in log.iter().filter(|c| c.seq > since && c.seq <= upto) {
                    latest.retain(|c| c.id != change.id);
                    latest.push(change);
                }
                let page: Vec<Value> = latest
                    .iter()
                    .skip(offset)
                    .take(page_size)
                    .map(|change| match render(&change.id) {
                        Some(value) if !change.removed => value,
                        _ => json!({ "id": change.id, "@removed": { "reason": "deleted" } }),
                    })
                    .collect();
                let reached = offset + page_size;

                if reached < latest.len() {
                    (
                        page,
                        format!(
                            "{link}?$skiptoken=c.{}.{since}.{upto}.{reached}",
                            self.epoch
                        ),
                    )
                } else {
                    (page, format!("{link}?$deltatoken=d.{}.{upto}", self.epoch))
                }
            }
            Some(_) => return Answer::Gone,
        };

        let key = if next.contains("$deltatoken=") {
            "@odata.deltaLink"
        } else {
            "@odata.nextLink"
        };
        let mut body = json!({ "@odata.context": "https://graph.microsoft.com/v1.0/$metadata#Collection(entity)", "value": entries });
        body[key] = Value::String(next);

        Answer::Page(body)
    }
}

fn strip_tags(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }

    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn recipients(list: &[(String, String)]) -> Value {
    list.iter()
        .map(|(name, address)| json!({ "emailAddress": { "name": name, "address": address } }))
        .collect()
}
