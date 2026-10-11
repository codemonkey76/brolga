use std::sync::Arc;

use brolga_graph::{
    DeltaItem, GraphClient, GraphError, MailFolder, Message, Next, Recipient, WELL_KNOWN_FOLDERS,
};
use brolga_mail::html_to_text;
use brolga_store::{AccountDb, Body, FolderChange, MessageChange, Round};
use serde_json::json;

use crate::SyncError;

/// How many times in a row the server may ask for a fresh start before the
/// sync gives up and reports it. One is ordinary; a stream of them is not.
const MAX_RESTARTS: u32 = 3;

/// What a sync did, for showing progress.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Synced {
    pub pages: u32,
    pub changed: u32,
    pub removed: u32,
    /// How many times the server asked for a fresh start.
    pub restarts: u32,
}

impl Synced {
    pub(crate) fn add(&mut self, other: Synced) {
        self.pages += other.pages;
        self.changed += other.changed;
        self.removed += other.removed;
        self.restarts += other.restarts;
    }
}

/// Syncs one account's mail.
pub struct MailSync {
    graph: Arc<GraphClient>,
    db: Arc<AccountDb>,
    page_size: u32,
}

impl MailSync {
    pub fn new(graph: Arc<GraphClient>, db: Arc<AccountDb>) -> Self {
        Self {
            graph,
            db,
            page_size: 100,
        }
    }

    /// Ask for this many messages a page. Smaller pages save progress more often.
    pub fn with_page_size(mut self, page_size: u32) -> Self {
        self.page_size = page_size.max(1);
        self
    }

    /// Sync the folder tree, then every folder's messages, the inbox first.
    pub async fn sync_all(&self) -> Result<Synced, SyncError> {
        let mut synced = self.sync_folders().await?;

        // `folders()` lists the well-known folders first, the inbox at the head.
        for folder in self.db.folders()? {
            synced.add(self.sync_messages(&folder.id).await?);
        }

        Ok(synced)
    }

    /// Bring the folder tree up to date.
    pub async fn sync_folders(&self) -> Result<Synced, SyncError> {
        let mut synced = Synced::default();
        let was_full_pass = self.db.folders_sync()?.reconciling;

        loop {
            let state = self.db.folders_sync()?;
            let page = match self.graph.mail_folders_delta(state.link.as_deref()).await {
                Ok(page) => page,
                Err(GraphError::SyncStateGone) => {
                    self.restarted(&mut synced, "the folder list")?;
                    self.db.restart_folders_sync()?;
                    continue;
                }
                Err(other) => return Err(other.into()),
            };

            let (changed, removed) = split(page.items, folder_change);
            let round = round(&page.next);
            self.db
                .apply_folders(&changed, &removed, page.next.url(), round)?;
            synced.pages += 1;
            synced.changed += count(&changed);
            synced.removed += count(&removed);

            // A full pass reports the state as it was when the pass began.
            // Whatever changed while it ran is in the round that follows, so
            // that round is run now and not left for the next sync.
            if round == Round::Finished && !state.reconciling {
                break;
            }
        }

        // Graph's v1.0 folder does not say which one is the inbox, so each
        // is asked for by its well-known name. Their ids do not change, so
        // this is done on a full pass, or while the inbox is not yet known.
        if was_full_pass
            || !self
                .db
                .folders()?
                .iter()
                .any(|f| f.well_known.as_deref() == Some("inbox"))
        {
            let mut known = Vec::new();
            for name in WELL_KNOWN_FOLDERS {
                if let Some(id) = self.graph.well_known_folder(name).await? {
                    known.push((*name, id));
                }
            }
            self.db.set_well_known(&known)?;
        }

        Ok(synced)
    }

    /// Bring one folder's messages up to date. A first sync carries on from
    /// wherever the last attempt reached.
    pub async fn sync_messages(&self, folder_id: &str) -> Result<Synced, SyncError> {
        let mut synced = Synced::default();

        loop {
            let state = self.db.messages_sync(folder_id)?;
            let page = match self
                .graph
                .messages_delta(folder_id, state.link.as_deref(), self.page_size)
                .await
            {
                Ok(page) => page,
                Err(GraphError::SyncStateGone) => {
                    self.restarted(&mut synced, "a folder")?;
                    self.db.restart_messages_sync(folder_id)?;
                    continue;
                }
                // The folder was deleted since the folder tree was read. Its
                // messages go when the next folder sync says it has gone.
                Err(GraphError::Refused { status: 404, .. }) => return Ok(synced),
                Err(other) => return Err(other.into()),
            };

            let (changed, removed) = split(page.items, message_change);
            let round = round(&page.next);
            self.db
                .apply_messages(folder_id, &changed, &removed, page.next.url(), round)?;
            synced.pages += 1;
            synced.changed += count(&changed);
            synced.removed += count(&removed);

            // As for folders: a full pass is followed at once by the round
            // that carries what changed while it ran.
            if round == Round::Finished && !state.reconciling {
                return Ok(synced);
            }
        }
    }

    /// Fetch the bodies of recent messages that are not here yet, newest
    /// first, so they can be read and searched with no connection. At most
    /// `limit` in one go: the rest wait for the next sync, which keeps any
    /// one sync short and well inside what Graph allows.
    ///
    /// `since` is a date, `YYYY-MM-DD`: messages received before it are left
    /// until they are opened.
    pub async fn download_bodies(&self, since: &str, limit: u32) -> Result<u32, SyncError> {
        let mut fetched = 0;

        for id in self.db.missing_bodies(since, limit)? {
            let body = match self.graph.message_body(&id).await {
                Ok(body) => Body {
                    content_type: body.content_type.to_ascii_lowercase(),
                    content: body.content,
                },
                // Deleted or moved out of the mailbox since it was listed.
                Err(GraphError::Refused { status: 404, .. }) => continue,
                Err(other) => return Err(other.into()),
            };
            let text = if body.content_type == "html" {
                html_to_text(&body.content)
            } else {
                body.content.clone()
            };
            self.db.save_body(&id, &body, &text)?;
            fetched += 1;
        }

        Ok(fetched)
    }

    fn restarted(&self, synced: &mut Synced, what: &str) -> Result<(), SyncError> {
        synced.restarts += 1;
        if synced.restarts > MAX_RESTARTS {
            return Err(SyncError::NeverSettled(what.to_owned()));
        }

        Ok(())
    }
}

fn round(next: &Next) -> Round {
    match next {
        Next::Page(_) => Round::Continues,
        Next::Done(_) => Round::Finished,
    }
}

fn count<T>(items: &[T]) -> u32 {
    u32::try_from(items.len()).unwrap_or(u32::MAX)
}

fn split<T, C>(items: Vec<DeltaItem<T>>, change: impl Fn(T) -> C) -> (Vec<C>, Vec<String>) {
    let mut changed = Vec::new();
    let mut removed = Vec::new();
    for item in items {
        match item {
            DeltaItem::Changed(item) => changed.push(change(item)),
            DeltaItem::Removed { id } => removed.push(id),
        }
    }

    (changed, removed)
}

fn folder_change(folder: MailFolder) -> FolderChange {
    FolderChange {
        id: folder.id,
        parent_id: folder.parent_folder_id,
        name: folder.display_name,
        unread: folder.unread_item_count,
        total: folder.total_item_count,
        hidden: folder.is_hidden,
    }
}

fn message_change(message: Message) -> MessageChange {
    let recipients = |list: Option<Vec<Recipient>>| {
        list.map(|list| {
            let list: Vec<_> = list
                .into_iter()
                .map(
                    |r| json!({ "name": r.email_address.name, "address": r.email_address.address }),
                )
                .collect();
            serde_json::Value::Array(list).to_string()
        })
    };
    let (from_name, from_address) = match message.from {
        Some(from) => (
            Some(from.email_address.name.unwrap_or_default()),
            Some(from.email_address.address.unwrap_or_default()),
        ),
        None => (None, None),
    };

    MessageChange {
        id: message.id,
        conversation_id: message.conversation_id,
        subject: message.subject,
        preview: message.body_preview,
        from_name,
        from_address,
        to_recipients: recipients(message.to_recipients),
        cc_recipients: recipients(message.cc_recipients),
        received_at: message.received_date_time,
        sent_at: message.sent_date_time,
        is_read: message.is_read,
        is_draft: message.is_draft,
        has_attachments: message.has_attachments,
        importance: message.importance,
        // The day it is due, where the server says: a flag that comes with
        // no day has none. An entry that does not mention the flag at all
        // leaves what is known alone.
        flag_due: message.flag.as_ref().map(|flag| {
            flag.due_date_time
                .as_ref()
                .and_then(|due| due.date_time.get(..10))
                .unwrap_or_default()
                .to_owned()
        }),
        flag_status: message.flag.and_then(|f| f.flag_status),
        categories: message
            .categories
            .map(|c| serde_json::Value::from(c).to_string()),
        inference: message.inference_classification,
    }
}
