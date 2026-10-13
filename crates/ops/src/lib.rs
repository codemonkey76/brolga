//! The outbox: every change a user makes, as a queued, idempotent operation.
//!
//! A change is made on this computer first (`brolga-store` does that, and
//! queues it in the same transaction). This crate sends what is queued. It
//! can be stopped at any point and started again: a change stays queued
//! until the server has answered for it, and sending one twice leaves the
//! mailbox as sending it once does.

use std::collections::HashSet;
use std::sync::Arc;

use brolga_graph::{Attachment, GraphClient, GraphError, Message};
use brolga_store::{AccountDb, Draft, Op, StoreError};
use serde_json::json;
use tokio::sync::Mutex;

/// Below this a file is attached in one request; from it up, in pieces.
const SMALL_FILE: usize = 3 * 1024 * 1024;

/// Said of a message that may have been sent: the request to send it
/// failed in a way that does not say, and the server either no longer has
/// the message to ask about or still shows it as a draft long after.
const UNCONFIRMED: &str =
    "It could not be confirmed that this was sent. Check Sent Items before sending again.";

/// Said of a message while it is being waited on to leave the drafts.
const CONFIRMING: &str = "Microsoft 365 did not say whether this was sent. Brolga is checking.";

/// How long a draft is watched after a request to send it that may have
/// been carried out. "message: send" (SOURCES.md) answers that a send is
/// accepted, not that it is done, and says nothing of how long a draft
/// stays one after that. Within this long, one that leaves the drafts was
/// sent. One still there after it is handed back to its writer, never sent
/// again unasked.
const CONFIRM_WITHIN: std::time::Duration = std::time::Duration::from_secs(300);

/// What became of the queue in one pass over it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Drained {
    /// The server has these.
    pub sent: u32,
    /// These could not be sent just now and are still queued.
    pub waiting: u32,
    /// The server will not take these. Each was taken back on this computer.
    pub refused: u32,
    /// Drafts whose copy on the server was brought up to date.
    pub saved: u32,
}

/// What became of a queued draft.
enum Sending {
    /// The server has sent it.
    Sent,
    /// Its writer took it back while it was in hand. Nothing was sent.
    TakenBack,
    /// A request to send it went out once and may have been carried out,
    /// and the server cannot say: it no longer has the message, or still
    /// shows it as a draft long after.
    Unconfirmed,
    /// A request to send it went out not long ago and may have been
    /// carried out. It is still a draft on the server, and is looked at
    /// again at the next pass.
    Confirming,
}

/// Sends one account's queued changes.
pub struct Outbox {
    db: Arc<AccountDb>,
    graph: Arc<GraphClient>,
    /// One pass at a time, so a change is not sent by two at once.
    draining: Mutex<()>,
    /// Windows' name for the zone a day with no time is a day of.
    zone: &'static str,
    /// How long a draft is watched after a request to send it that may
    /// have been carried out.
    confirm_within: std::time::Duration,
}

impl Outbox {
    /// An outbox for a person in this computer's time zone.
    pub fn new(graph: Arc<GraphClient>, db: Arc<AccountDb>) -> Self {
        let zone = jiff::tz::TimeZone::system()
            .iana_name()
            .and_then(windows_zone)
            .unwrap_or("UTC");

        Self {
            db,
            graph,
            draining: Mutex::new(()),
            zone,
            confirm_within: CONFIRM_WITHIN,
        }
    }

    /// The same outbox, watching for this long a draft whose sending could
    /// not be confirmed before it is handed back to its writer.
    pub fn confirming_within(mut self, long: std::time::Duration) -> Self {
        self.confirm_within = long;
        self
    }

    /// The same outbox for a person in another time zone, named as the
    /// zone database or as Windows names it. A name that is neither
    /// changes nothing.
    pub fn in_zone(mut self, name: &str) -> Self {
        if let Some(zone) = windows_zone(name) {
            self.zone = zone;
        }
        self
    }

    /// Send what is queued, oldest first. The changes to one message are
    /// sent in the order they were made: when one cannot be sent now, the
    /// later ones for that message wait behind it.
    pub async fn drain(&self) -> Result<Drained, StoreError> {
        let _one_at_a_time = self.draining.lock().await;
        let mut drained = Drained::default();
        let mut held_up: HashSet<String> = HashSet::new();
        let mut gone: HashSet<String> = HashSet::new();

        for op in self.db.queued_ops()? {
            // Forgotten earlier in this pass, and its changes with it.
            if gone.contains(&op.message_id) {
                drained.sent += 1;
                continue;
            }
            if held_up.contains(&op.message_id) {
                drained.waiting += 1;
                continue;
            }

            let mut result = self.send(&op).await;
            // Something is not there. If it is the message, there is nothing
            // left to change, and it is forgotten here: it may have been
            // moved on this computer to where no sync would look for it. If
            // the message is there, it is something else the change needs
            // (the folder it was to go to), and the change is refused.
            if let Err(GraphError::Refused { status: 404, .. }) = &result {
                match self.graph.message_folder(&op.message_id).await {
                    Err(GraphError::Refused { status: 404, .. }) => {
                        self.db.forget_message(&op.message_id)?;
                        gone.insert(op.message_id);
                        drained.sent += 1;
                        continue;
                    }
                    Ok(_) => {}
                    Err(other) => result = Err(other),
                }
            }

            match result {
                Ok(()) => {
                    self.db.op_sent(op.id)?;
                    drained.sent += 1;
                }
                Err(error) if will_pass(&error) => {
                    self.db.op_waits(op.id, &error.to_string())?;
                    held_up.insert(op.message_id);
                    drained.waiting += 1;
                }
                Err(error) => {
                    self.db.op_refused(op.id, &error.to_string())?;
                    drained.refused += 1;
                }
            }
        }

        self.discard_copies().await?;
        drained.saved = self.save_drafts().await?;

        for listed in self.db.queued_drafts()? {
            // As it is now: it may have been taken back since the list was read.
            let Some(draft) = self
                .db
                .draft(&listed.id)?
                .filter(|draft| draft.state == "queued")
            else {
                continue;
            };

            match self.send_draft(&draft).await {
                Ok(Sending::Sent) => {
                    // An answer or a forward marks the message it was made
                    // from, here and now: the server does not say so in a sync.
                    let done = match draft.kind.as_str() {
                        "reply" => Some("replied"),
                        "reply_all" => Some("replied_all"),
                        "forward" => Some("forwarded"),
                        _ => None,
                    };
                    if let (Some(done), Some(source)) = (done, &draft.source_id) {
                        self.db.mark_last_action(source, done)?;
                    }
                    self.db.draft_sent(&draft.id)?;
                    drained.sent += 1;
                }
                Ok(Sending::TakenBack) => {}
                // Kept, and handed back to its writer to look and decide:
                // sending it again unasked could send it twice.
                Ok(Sending::Unconfirmed) => {
                    self.db.draft_refused(&draft.id, UNCONFIRMED)?;
                    drained.refused += 1;
                }
                Ok(Sending::Confirming) => {
                    self.db.draft_waits(&draft.id, CONFIRMING)?;
                    drained.waiting += 1;
                }
                Err(error) if will_pass(&error) => {
                    self.db.draft_waits(&draft.id, &error.to_string())?;
                    drained.waiting += 1;
                }
                Err(error) => {
                    self.db.draft_refused(&draft.id, &error.to_string())?;
                    drained.refused += 1;
                }
            }
        }

        Ok(drained)
    }

    /// Send a queued draft, exactly once.
    ///
    /// It is made a draft on the server, given its words and recipients,
    /// and then sent. Each step is written down before the next, so this can
    /// be stopped anywhere and run again. The one step that cannot simply
    /// be run again is the sending itself. A request to send that
    /// certainly was not carried out is made again. One that may have been
    /// is never made again from here: the draft is looked at, and if it is
    /// no longer a draft on the server it has been sent. If it is still one
    /// after a while, nothing documented says whether it is about to go,
    /// so it is handed back to its writer to look and decide.
    ///
    /// The draft in hand is the draft as it was when this began. Its writer
    /// may take it back at any moment before the request to send goes out,
    /// and that request goes only if they have not.
    async fn send_draft(&self, draft: &Draft) -> Result<Sending, GraphError> {
        if draft.send_tried
            && let Some(server_id) = &draft.server_id
        {
            return Ok(match self.graph.is_draft(server_id).await? {
                Some(false) => Sending::Sent,
                // Still a draft. The request may be waiting its turn there.
                Some(true) => {
                    let ago = self.db.draft_send_tried_ago(&draft.id).map_err(kept)?;
                    let recent = ago.is_some_and(|ago| {
                        u64::try_from(ago).is_ok_and(|ago| ago < self.confirm_within.as_secs())
                    });
                    if recent {
                        Sending::Confirming
                    } else {
                        Sending::Unconfirmed
                    }
                }
                // Gone, sent or not. Only the message itself could have said.
                None => Sending::Unconfirmed,
            });
        }

        let server_id = match self.put_on_server(draft).await {
            Ok(Some(server_id)) => server_id,
            Ok(None) => return Ok(Sending::TakenBack),
            // The server's copy has gone: thrown away in another mail
            // program, most likely. The message is still wanted, so it is
            // made again and sent from there.
            Err(GraphError::Refused { status: 404, .. }) if draft.server_id.is_some() => {
                self.db.draft_unlinked(&draft.id).map_err(kept)?;
                let mut fresh = draft.clone();
                fresh.server_id = None;
                fresh.quoted = None;
                match self.put_on_server(&fresh).await? {
                    Some(server_id) => server_id,
                    None => return Ok(Sending::TakenBack),
                }
            }
            Err(other) => return Err(other),
        };

        self.level_files(draft, &server_id).await?;

        if !self
            .db
            .draft_send_tried(&draft.id, draft.rev)
            .map_err(kept)?
        {
            return Ok(Sending::TakenBack);
        }
        if let Err(error) = self.graph.send_draft(&server_id).await {
            // Turned away before anything was done: it has not been sent,
            // and may be sent at the next pass. Any other failure leaves
            // it marked as tried, to be looked at and not sent again.
            if error.not_carried_out() {
                self.db.draft_send_not_made(&draft.id).map_err(kept)?;
            }
            return Err(error);
        }

        Ok(Sending::Sent)
    }

    /// Bring the server's copy of a draft up to what is here, making it
    /// first if there is none. Gives back its id there, or nothing if the
    /// draft was thrown away here while its copy was being made: the copy
    /// is then not wanted either, and is noted to be thrown away.
    async fn put_on_server(&self, draft: &Draft) -> Result<Option<String>, GraphError> {
        let (server_id, quoted) = match &draft.server_id {
            Some(server_id) => (server_id.clone(), draft.quoted.clone()),
            None => {
                let created = match (draft.kind.as_str(), &draft.source_id) {
                    ("reply", Some(source)) => {
                        self.graph.create_draft_from(source, "createReply").await?
                    }
                    ("reply_all", Some(source)) => {
                        self.graph
                            .create_draft_from(source, "createReplyAll")
                            .await?
                    }
                    ("forward", Some(source)) => {
                        self.graph
                            .create_draft_from(source, "createForward")
                            .await?
                    }
                    _ => self.new_draft(draft).await?,
                };
                // What the server put in the draft is the original, quoted
                // as mail programs expect. The words written go above it.
                let quoted = created
                    .body
                    .map(|body| body.content)
                    .filter(|q| !q.trim().is_empty());
                if !self
                    .db
                    .draft_on_server(&draft.id, &created.id, quoted.as_deref())
                    .map_err(kept)?
                {
                    self.db.discard_on_server(&created.id).map_err(kept)?;
                    return Ok(None);
                }
                (created.id, quoted)
            }
        };

        let mut content = json!({
            "subject": draft.subject,
            "body": { "contentType": "html", "content": html(&draft.body, &draft.format, quoted.as_deref()) },
            "toRecipients": recipients(&draft.to_recipients),
            "ccRecipients": recipients(&draft.cc_recipients),
            "bccRecipients": recipients(&draft.bcc_recipients),
        });
        // Under another name, where one was chosen ("Update message",
        // SOURCES.md: `from` may be set on a draft). Whether it goes as that
        // mailbox or on its behalf is for the server, by what was granted.
        if !draft.from_address.is_empty() {
            content["from"] = json!({ "emailAddress": { "address": draft.from_address } });
        }
        self.graph.update_message(&server_id, &content).await?;

        Ok(Some(server_id))
    }

    /// Make the server's copy of a new message, once.
    ///
    /// Making a draft is not safe to ask for twice, so the copy is marked
    /// with something chosen and written down before it is asked for. If
    /// the answer never came, the mark is looked for at the next pass, and
    /// the copy that was made is taken up where a second would have been
    /// made beside it.
    ///
    /// An answer or a forward cannot be marked: `createReply` and its like
    /// take nothing documented that would carry a mark ("message:
    /// createReply", SOURCES.md), so a copy of one of those whose making
    /// was never heard of stays in the mailbox's drafts, and a second is
    /// made. Nothing tells it from a draft the person began elsewhere, and
    /// it is not thrown away on a guess.
    async fn new_draft(&self, draft: &Draft) -> Result<Message, GraphError> {
        let mark = match self.db.draft_create_marker(&draft.id).map_err(kept)? {
            Some(mark) => {
                let mut made = self.graph.drafts_marked(&mark).await?.into_iter();
                if let Some(id) = made.next() {
                    // More than one of the same mark is more than was ever
                    // asked for. They are this draft's all the same, and
                    // one is enough.
                    for spare in made {
                        self.db.discard_on_server(&spare).map_err(kept)?;
                    }
                    return self.graph.draft(&id).await;
                }
                mark
            }
            None => {
                let mut bytes = [0_u8; 16];
                getrandom::fill(&mut bytes).map_err(|e| GraphError::Decode(e.to_string()))?;
                let mark: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                self.db
                    .set_draft_create_marker(&draft.id, &mark)
                    .map_err(kept)?;
                mark
            }
        };

        self.graph
            .create_draft(&json!({
                "singleValueExtendedProperties": [{ "id": brolga_graph::DRAFT_MARK, "value": mark }]
            }))
            .await
    }

    /// Keep the server's Drafts folder level with what is being written
    /// here, so a draft shows in other mail programs. A draft that cannot
    /// be saved there now is saved at the next pass; it is safe here
    /// meanwhile. Gives back how many were saved.
    async fn save_drafts(&self) -> Result<u32, StoreError> {
        let mut saved = 0;
        for draft in self.db.drafts_to_upload()? {
            match self.put_on_server(&draft).await {
                Ok(Some(_)) => {
                    self.db.draft_uploaded(&draft.id, draft.rev)?;
                    saved += 1;
                }
                // The server's copy has gone. The words are still here, and
                // get a new copy at the next pass.
                Err(GraphError::Refused { status: 404, .. }) if draft.server_id.is_some() => {
                    self.db.draft_unlinked(&draft.id)?;
                }
                Ok(None) | Err(_) => {}
            }
        }

        Ok(saved)
    }

    /// Move to Deleted Items the copies of drafts on the server that are
    /// no longer wanted: of a draft thrown away here before its copy was
    /// known of here as a message, or while the copy was being made. One
    /// that cannot be moved now is tried again at the next pass.
    async fn discard_copies(&self) -> Result<(), StoreError> {
        let deleted = self.db.folder_id("deleteditems")?;
        for (server_id, attempts) in self.db.draft_discards()? {
            // A move is not safe to send twice blindly: where an earlier try
            // may have arrived, the copy's folder is looked at first.
            let moved = if attempts > 0 {
                match self.graph.message_folder(&server_id).await {
                    Ok(folder) if folder.is_some() && folder == deleted => Ok(()),
                    Ok(_) => self.graph.move_message(&server_id, "deleteditems").await,
                    Err(error) => Err(error),
                }
            } else {
                self.graph.move_message(&server_id, "deleteditems").await
            };

            match moved {
                Err(error) if will_pass(&error) => self.db.draft_discard_waits(&server_id)?,
                // Moved, or already gone, or not to be moved at all.
                Ok(()) | Err(_) => self.db.draft_discard_done(&server_id)?,
            }
        }

        Ok(())
    }

    /// Make the files on the server's copy of a draft the files the draft
    /// has here: those taken off here are taken off there, and those not
    /// yet there are put there, each once. Attaching is not safe to repeat
    /// blindly, so where an earlier request for a file may have arrived,
    /// the server's list is looked at first.
    ///
    /// What was attached to the copy before any file went to it from here
    /// (on a draft begun in another mail program, or the original's files
    /// on a forward) is noted first, and is never counted as a file from
    /// here nor taken off.
    async fn level_files(&self, draft: &Draft, server_id: &str) -> Result<(), GraphError> {
        let mut files = self.db.draft_files(&draft.id).map_err(kept)?;
        let removed = self.db.draft_files_removed(&draft.id).map_err(kept)?;
        if removed.is_empty() && files.iter().all(|f| f.uploaded) {
            return Ok(());
        }

        let before = match self.db.draft_attachments_before(&draft.id).map_err(kept)? {
            Some(before) => before,
            None => {
                let before: Vec<String> = self
                    .graph
                    .attachments(server_id)
                    .await?
                    .into_iter()
                    .map(|a| a.id)
                    .collect();
                self.db
                    .set_draft_attachments_before(&draft.id, &before)
                    .map_err(kept)?;
                before
            }
        };
        let unsure = !removed.is_empty() || files.iter().any(|f| f.upload_tried && !f.uploaded);
        let mut there = if unsure {
            self.graph.attachments(server_id).await?
        } else {
            Vec::new()
        };

        for (removal, attachment_id, name) in removed {
            let known = attachment_id.filter(|id| there.iter().any(|a| a.id == *id));
            let going: Vec<String> = match known {
                Some(id) => vec![id],
                // Its id is not known, or is not one the server lists. Then
                // everything of its name that came from here and is not
                // known to be another file goes, and any file of that name
                // still wanted, whose id is not known either, goes up again.
                None => {
                    let mut going = Vec::new();
                    for a in &there {
                        if a.name.as_deref() == Some(name.as_str()) && from_here(a, &before, &files)
                        {
                            going.push(a.id.clone());
                        }
                    }
                    for file in &files {
                        if file.name == name && file.attachment_id.is_none() && file.upload_tried {
                            self.db.draft_file_not_uploaded(file.id).map_err(kept)?;
                        }
                    }
                    going
                }
            };
            for id in going {
                self.graph.delete_attachment(server_id, &id).await?;
                there.retain(|a| a.id != id);
            }
            self.db.draft_file_removal_done(removal).map_err(kept)?;
            files = self.db.draft_files(&draft.id).map_err(kept)?;
        }

        for file in files.iter().filter(|f| !f.uploaded) {
            if file.upload_tried {
                // One the server already has is claimed, so that two files
                // alike in name are each attached. It must be at least the
                // file's size: the server counts more than the file's own
                // bytes ("Attach large files to Outlook messages or events"
                // shows a file of 3,483,322 bytes listed as 3,640,066), so
                // the sizes cannot be asked to be the same.
                let found = there.iter().position(|a| {
                    a.name.as_deref() == Some(file.name.as_str())
                        && a.size.is_some_and(|size| size >= file.size)
                        && from_here(a, &before, &files)
                });
                if let Some(found) = found {
                    let claimed = there.remove(found);
                    self.db
                        .draft_file_uploaded(file.id, Some(&claimed.id))
                        .map_err(kept)?;
                    continue;
                }
            }
            let Some(data) = self.db.draft_file_data(file.id).map_err(kept)? else {
                continue;
            };
            self.db.draft_file_upload_tried(file.id).map_err(kept)?;
            // "Add attachment" takes a file of under 3 MB; a larger one
            // goes up in pieces.
            let content_id = file.content_id.as_deref();
            let attachment_id = if data.len() < SMALL_FILE {
                self.graph
                    .add_attachment(server_id, &file.name, &file.content_type, &data, content_id)
                    .await?
            } else {
                self.graph
                    .upload_attachment(server_id, &file.name, &data, content_id)
                    .await?
            };
            self.db
                .draft_file_uploaded(file.id, attachment_id.as_deref())
                .map_err(kept)?;
        }

        Ok(())
    }

    async fn send(&self, op: &Op) -> Result<(), GraphError> {
        match op.kind.as_str() {
            // Setting a property twice leaves it as setting it once does.
            "is_read" => {
                let changes = json!({ "isRead": op.value == "1" });
                self.graph.update_message(&op.message_id, &changes).await
            }
            "flag_status" => {
                let changes = json!({ "flag": { "flagStatus": op.value } });
                self.graph.update_message(&op.message_id, &changes).await
            }
            // The day was taken off the flag. The flag is sent as it now
            // stands here, with no dates. "followupFlag resource type" does
            // not say how a date is taken away; null is how a property is
            // given no value in an update.
            "flag_due" if op.value.is_empty() => {
                let Some(message) = self.db.message(&op.message_id).map_err(kept)? else {
                    return Ok(());
                };
                let changes = json!({
                    "flag": { "flagStatus": message.flag_status, "startDateTime": null, "dueDateTime": null }
                });
                self.graph.update_message(&op.message_id, &changes).await
            }
            // "followupFlag resource type": a due date must come with a
            // start. A day with no time is given as its first moment by
            // the clock where its writer is, with the zone named as
            // "dateTimeTimeZone resource type" asks, so that it is that
            // day in their other mail programs too.
            "flag_due" => {
                let day =
                    json!({ "dateTime": format!("{}T00:00:00", op.value), "timeZone": self.zone });
                let changes = json!({
                    "flag": { "flagStatus": "flagged", "startDateTime": day, "dueDateTime": day }
                });
                self.graph.update_message(&op.message_id, &changes).await
            }
            "categories" => {
                let names = category_names(&op.value)?;
                self.graph
                    .update_message(&op.message_id, &json!({ "categories": names }))
                    .await
            }
            "folder_id" => self.move_message(op).await,
            other => Err(GraphError::Decode(format!(
                "a queued change of a kind this version does not know: {other}"
            ))),
        }
    }

    /// A move is not safe to send twice blindly. When an earlier try may
    /// have reached the server, the message's folder is looked at first,
    /// and the move is sent only if it has not already been made.
    async fn move_message(&self, op: &Op) -> Result<(), GraphError> {
        if op.attempts > 0
            && self.graph.message_folder(&op.message_id).await?.as_deref()
                == Some(op.value.as_str())
        {
            return Ok(());
        }

        self.graph.move_message(&op.message_id, &op.value).await
    }
}

/// A failure to read or write what is kept here, as the error the sending
/// of a change ends with.
fn kept(error: StoreError) -> GraphError {
    GraphError::Decode(error.to_string())
}

/// Windows' name for a time zone, which is the one Microsoft 365 is sure
/// to take ("dateTimeTimeZone resource type", SOURCES.md).
fn windows_zone(name: &str) -> Option<&'static str> {
    brolga_calendar::iana_zone(name)
        .and_then(brolga_calendar::windows_zone)
        .or_else(|| brolga_calendar::windows_zone(name))
}

/// Whether an attachment on the server's copy of a draft may be a file put
/// there from here whose id is not known: it was not there before the
/// first file went, and it is not known to be another of the draft's files.
fn from_here(
    attachment: &Attachment,
    before: &[String],
    files: &[brolga_store::DraftFile],
) -> bool {
    !before.contains(&attachment.id)
        && !files
            .iter()
            .any(|f| f.attachment_id.as_deref() == Some(attachment.id.as_str()))
}

/// A message's categories as they were queued: a JSON array of names. What
/// cannot be read as that is refused. Sending no names in its place would
/// take every category off the message.
fn category_names(queued: &str) -> Result<serde_json::Value, GraphError> {
    serde_json::from_str::<Vec<String>>(queued)
        .map(serde_json::Value::from)
        .map_err(|_| {
            GraphError::Decode("a queued change of categories that cannot be read".to_owned())
        })
}

/// Whether the same request may well succeed later: the network is down,
/// the service is busy, or the account has to sign in again first.
fn will_pass(error: &GraphError) -> bool {
    error.is_transient()
        || matches!(
            error,
            GraphError::Token(_) | GraphError::Unauthorized { .. }
        )
}

/// Recipients as kept (`[{name, address}]`) in the shape Graph takes.
fn recipients(kept: &str) -> serde_json::Value {
    let list: Vec<serde_json::Value> = serde_json::from_str(kept).unwrap_or_default();

    list.iter()
        .filter_map(|each| {
            let address = each["address"].as_str()?.trim();
            (!address.is_empty()).then(|| {
                json!({ "emailAddress": { "name": each["name"].as_str().unwrap_or(address), "address": address } })
            })
        })
        .collect()
}

/// A draft's body as the HTML that is sent.
///
/// Plain words become a paragraph each, with line breaks kept and nothing
/// typed taken as markup. A formatted body is cleaned on its way out as it
/// was on its way in: only plain tags leave, whatever put it there.
/// Whatever is quoted goes underneath as the server wrote it.
fn html(body: &str, format: &str, quoted: Option<&str>) -> String {
    let mut out = String::from("<html><body>");
    if format == "html" {
        out.push_str(&brolga_mail::sanitise_html(body, brolga_mail::RemoteContent::Blocked).html);
    } else {
        out.push_str(&brolga_mail::text_to_paragraphs(body));
    }
    if let Some(quoted) = quoted {
        out.push_str(quoted);
    }
    out.push_str("</body></html>");

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_words_become_plain_paragraphs_and_nothing_typed_becomes_markup() {
        assert_eq!(
            html("Hi Sam,\n\nTwo lines\nhere <b>&</b>.\n", "text", None),
            "<html><body><p>Hi Sam,</p><p>Two lines<br>here &lt;b&gt;&amp;&lt;/b&gt;.</p></body></html>"
        );
        assert_eq!(
            html("Yes.", "text", Some("<hr><p>Original</p>")),
            "<html><body><p>Yes.</p><hr><p>Original</p></body></html>"
        );
    }

    #[test]
    fn a_formatted_body_goes_out_with_its_formatting_and_nothing_else() {
        let sent = html(
            "<p>Hi <b>Sam</b>, see <a href=\"https://example.test/a\">this</a>.</p><ul><li>One</li></ul>\
             <script>evil()</script><p style=\"position:fixed\" onclick=\"evil()\">x</p><img src=\"https://tracker.test/p.gif\">",
            "html",
            Some("<hr><p>Original</p>"),
        );

        assert!(
            sent.contains("<p>Hi <b>Sam</b>, see <a href=\"https://example.test/a\""),
            "{sent}"
        );
        assert!(sent.contains("<ul><li>One</li></ul>"), "{sent}");
        assert!(
            !sent.contains("script")
                && !sent.contains("onclick")
                && !sent.contains("style")
                && !sent.contains("tracker"),
            "{sent}"
        );
        assert!(
            sent.ends_with("<hr><p>Original</p></body></html>"),
            "{sent}"
        );
    }

    #[test]
    fn categories_that_cannot_be_read_are_refused_and_not_sent_as_none() {
        assert_eq!(
            category_names(r#"["Red","Work"]"#).ok(),
            Some(json!(["Red", "Work"]))
        );
        assert_eq!(category_names("[]").ok(), Some(json!([])));
        assert!(category_names("").is_err());
        assert!(category_names(r#"["Red""#).is_err());
        assert!(category_names(r#"{"Red":1}"#).is_err());
    }

    #[test]
    fn recipients_without_an_address_are_left_out() {
        let kept = r#"[{"name":"Sam","address":"sam@example.test"},{"name":"Nobody","address":" "},{"address":"jo@example.test"}]"#;

        assert_eq!(
            recipients(kept),
            json!([
                { "emailAddress": { "name": "Sam", "address": "sam@example.test" } },
                { "emailAddress": { "name": "jo@example.test", "address": "jo@example.test" } }
            ])
        );
    }
}
