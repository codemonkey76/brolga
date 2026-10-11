//! The outbox: every change a user makes, as a queued, idempotent operation.
//!
//! A change is made on this computer first (`brolga-store` does that, and
//! queues it in the same transaction). This crate sends what is queued. It
//! can be stopped at any point and started again: a change stays queued
//! until the server has answered for it, and sending one twice leaves the
//! mailbox as sending it once does.

use std::collections::HashSet;
use std::sync::Arc;

use brolga_graph::{GraphClient, GraphError};
use brolga_store::{AccountDb, Draft, Op, StoreError};
use serde_json::json;
use tokio::sync::Mutex;

/// What became of the queue in one pass over it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Drained {
    /// The server has these.
    pub sent: u32,
    /// These could not be sent just now and are still queued.
    pub waiting: u32,
    /// The server will not take these. Each was taken back on this computer.
    pub refused: u32,
}

/// Sends one account's queued changes.
pub struct Outbox {
    db: Arc<AccountDb>,
    graph: Arc<GraphClient>,
    /// One pass at a time, so a change is not sent by two at once.
    draining: Mutex<()>,
}

impl Outbox {
    pub fn new(graph: Arc<GraphClient>, db: Arc<AccountDb>) -> Self {
        Self {
            db,
            graph,
            draining: Mutex::new(()),
        }
    }

    /// Send what is queued, oldest first. The changes to one message are
    /// sent in the order they were made: when one cannot be sent now, the
    /// later ones for that message wait behind it.
    pub async fn drain(&self) -> Result<Drained, StoreError> {
        let _one_at_a_time = self.draining.lock().await;
        let mut drained = Drained::default();
        let mut held_up: HashSet<String> = HashSet::new();

        for op in self.db.queued_ops()? {
            if held_up.contains(&op.message_id) {
                drained.waiting += 1;
                continue;
            }

            match self.send(&op).await {
                Ok(()) => {
                    self.db.op_sent(op.id)?;
                    drained.sent += 1;
                }
                // The message is gone from the server. There is nothing left
                // to change, and the next sync takes it from here too.
                Err(GraphError::Refused { status: 404, .. }) => {
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

        for draft in self.db.queued_drafts()? {
            match self.send_draft(&draft).await {
                Ok(()) => {
                    self.db.draft_sent(&draft.id)?;
                    drained.sent += 1;
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
    /// be run again is the sending itself: where a request to send may have
    /// reached the server, the draft is looked at first, and if it is no
    /// longer a draft there it has been sent and is not sent again.
    async fn send_draft(&self, draft: &Draft) -> Result<(), GraphError> {
        if draft.send_tried
            && let Some(server_id) = &draft.server_id
            && self.graph.is_draft(server_id).await? != Some(true)
        {
            return Ok(());
        }

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
                    _ => self.graph.create_draft(&json!({})).await?,
                };
                // What the server put in the draft is the original, quoted
                // as mail programs expect. The words written go above it.
                let quoted = created
                    .body
                    .map(|body| body.content)
                    .filter(|q| !q.trim().is_empty());
                self.db
                    .draft_on_server(&draft.id, &created.id, quoted.as_deref())
                    .map_err(|e| GraphError::Decode(e.to_string()))?;
                (created.id, quoted)
            }
        };

        let content = json!({
            "subject": draft.subject,
            "body": { "contentType": "html", "content": html(&draft.body, quoted.as_deref()) },
            "toRecipients": recipients(&draft.to_recipients),
            "ccRecipients": recipients(&draft.cc_recipients),
            "bccRecipients": recipients(&draft.bcc_recipients),
        });
        self.graph.update_message(&server_id, &content).await?;

        self.attach_files(draft, &server_id).await?;

        self.db
            .draft_send_tried(&draft.id)
            .map_err(|e| GraphError::Decode(e.to_string()))?;
        self.graph.send_draft(&server_id).await
    }

    /// Put a draft's files on the server's copy of it, each once. Attaching
    /// is not safe to repeat blindly, so where an earlier request for a file
    /// may have arrived, the server's list is looked at first.
    async fn attach_files(&self, draft: &Draft, server_id: &str) -> Result<(), GraphError> {
        let kept = |e: StoreError| GraphError::Decode(e.to_string());
        let files = self.db.draft_files(&draft.id).map_err(kept)?;
        let unsure = files.iter().any(|f| f.upload_tried && !f.uploaded);
        let mut there = if unsure {
            self.graph.attachments(server_id).await?
        } else {
            Vec::new()
        };

        for file in files.iter().filter(|f| !f.uploaded) {
            if file.upload_tried {
                // One the server already has is claimed, so that two files
                // alike in name and size are each attached.
                let found = there.iter().position(|a| {
                    a.name.as_deref() == Some(file.name.as_str()) && a.size.is_some()
                });
                if let Some(found) = found {
                    there.remove(found);
                    self.db.draft_file_uploaded(file.id).map_err(kept)?;
                    continue;
                }
            }
            let Some(data) = self.db.draft_file_data(file.id).map_err(kept)? else {
                continue;
            };
            self.db.draft_file_upload_tried(file.id).map_err(kept)?;
            self.graph
                .add_attachment(server_id, &file.name, &file.content_type, &data)
                .await?;
            self.db.draft_file_uploaded(file.id).map_err(kept)?;
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

/// The words as typed, as the plain HTML a mail program shows well: a
/// paragraph for each, line breaks kept, nothing else. Whatever is quoted
/// goes underneath as the server wrote it.
fn html(typed: &str, quoted: Option<&str>) -> String {
    let mut out = String::from("<html><body>");
    for paragraph in typed.replace("\r\n", "\n").split("\n\n") {
        if paragraph.trim().is_empty() {
            continue;
        }
        out.push_str("<p>");
        out.push_str(&escape(paragraph).replace('\n', "<br>"));
        out.push_str("</p>");
    }
    if let Some(quoted) = quoted {
        out.push_str(quoted);
    }
    out.push_str("</body></html>");

    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_words_become_plain_paragraphs_and_nothing_typed_becomes_markup() {
        assert_eq!(
            html("Hi Sam,\n\nTwo lines\nhere <b>&</b>.\n", None),
            "<html><body><p>Hi Sam,</p><p>Two lines<br>here &lt;b&gt;&amp;&lt;/b&gt;.<br></p></body></html>"
        );
        assert_eq!(
            html("Yes.", Some("<hr><p>Original</p>")),
            "<html><body><p>Yes.</p><hr><p>Original</p></body></html>"
        );
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
