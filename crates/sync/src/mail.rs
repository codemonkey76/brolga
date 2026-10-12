use std::sync::{Arc, Mutex, PoisonError};

use brolga_graph::{
    DeltaItem, GraphClient, GraphError, MailFolder, Message, Next, Recipient, WELL_KNOWN_FOLDERS,
};
use brolga_mail::html_to_text;
use brolga_store::{AccountDb, Body, FolderChange, MessageChange, Round};
use serde_json::json;

use crate::{Outcome, SyncError};

/// How many times in a row the server may ask for a fresh start before the
/// sync gives up and reports it. One is ordinary; a stream of them is not.
const MAX_RESTARTS: u32 = 3;

/// Where the asking after answered and forwarded messages has got to: the
/// page to ask next while a pass is under way, when that pass began, and
/// the moment the last finished pass covered up to.
const ACTIONS_LINK: &str = "last_actions.link";
const ACTIONS_BEGAN: &str = "last_actions.began";
const ACTIONS_SINCE: &str = "last_actions.since";

/// What a sync did, for showing progress.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Synced {
    pub pages: u32,
    pub changed: u32,
    pub removed: u32,
    /// How many times the server asked for a fresh start.
    pub restarts: u32,
    /// How many items came with a part that could not be read, and were
    /// kept without that part.
    pub damaged: u32,
}

impl Synced {
    pub(crate) fn add(&mut self, other: Synced) {
        self.pages += other.pages;
        self.changed += other.changed;
        self.removed += other.removed;
        self.restarts += other.restarts;
        self.damaged += other.damaged;
    }
}

/// What a sync of mail is busy with, for whoever is waiting on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Doing {
    /// Reading the folder tree.
    Folders,
    /// Fetching the messages of the folder with this id.
    Messages(String),
}

/// Syncs one account's mail.
pub struct MailSync {
    graph: Arc<GraphClient>,
    db: Arc<AccountDb>,
    page_size: u32,
    doing: Mutex<Option<Doing>>,
}

impl MailSync {
    pub fn new(graph: Arc<GraphClient>, db: Arc<AccountDb>) -> Self {
        Self {
            graph,
            db,
            page_size: 100,
            doing: Mutex::new(None),
        }
    }

    /// Ask for this many messages a page. Smaller pages save progress more often.
    pub fn with_page_size(mut self, page_size: u32) -> Self {
        self.page_size = page_size.max(1);
        self
    }

    /// What a sync that is running has reached. Nothing when none is.
    pub fn doing(&self) -> Option<Doing> {
        self.doing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn now(&self, doing: Option<Doing>) {
        *self.doing.lock().unwrap_or_else(PoisonError::into_inner) = doing;
    }

    /// Sync the folder tree, then every folder's messages, the inbox first.
    /// Gives the first thing that went wrong, if anything did, having
    /// still synced every folder that could be.
    pub async fn sync_all(&self) -> Result<Synced, SyncError> {
        self.sync_each().await.into_result()
    }

    /// Sync the folder tree, then every folder's messages, the inbox first.
    /// A folder that cannot be synced is noted and does not stop the ones
    /// after it, unless the trouble is the whole account's.
    pub async fn sync_each(&self) -> Outcome {
        let mut outcome = Outcome::default();
        self.all(&mut outcome).await;
        // However it ended, it is busy with nothing now.
        self.now(None);

        outcome
    }

    async fn all(&self, outcome: &mut Outcome) {
        self.now(Some(Doing::Folders));
        // Without the folder tree, the folders known from before are still
        // synced: what is new in them is no less wanted. But where the
        // server could not be had for the very first thing asked of it,
        // it is not then asked for every folder in turn.
        match self.sync_folders().await {
            Ok(synced) => outcome.done(synced),
            Err(error) => {
                let passing = error.is_transient();
                if outcome.fail("The list of folders", error) || passing {
                    outcome.stopped = true;
                    return;
                }
            }
        }

        // `folders()` lists the well-known folders first, the inbox at the head.
        let folders = match self.db.folders() {
            Ok(folders) => folders,
            Err(error) => {
                outcome.fail("The list of folders", error.into());
                return;
            }
        };
        let mut failed = std::collections::HashSet::new();

        // First the newest page of every folder nothing has been fetched of
        // yet, so that no folder sits empty while a large one ahead of it is
        // fetched whole. A page is asked for newest first.
        for folder in &folders {
            self.now(Some(Doing::Messages(folder.id.clone())));
            match self.first_page(&folder.id).await {
                Ok(synced) => outcome.done(synced),
                Err(error) => {
                    failed.insert(folder.id.as_str());
                    if outcome.fail(&folder.name, error) {
                        return;
                    }
                }
            }
        }

        // Then each folder whole, carrying on from the page it has. One
        // that has just failed is left for the next sync.
        for folder in &folders {
            if failed.contains(folder.id.as_str()) {
                continue;
            }
            self.now(Some(Doing::Messages(folder.id.clone())));
            match self.sync_messages(&folder.id).await {
                Ok(synced) => outcome.done(synced),
                Err(error) => {
                    if outcome.fail(&folder.name, error) {
                        return;
                    }
                }
            }
        }
    }

    /// Fetch the newest page of a folder nothing has been fetched of yet.
    async fn first_page(&self, folder_id: &str) -> Result<Synced, SyncError> {
        let mut synced = Synced::default();
        let state = self.db.messages_sync(folder_id)?;
        if state.link.is_none() && !state.complete {
            self.messages_page(folder_id, &mut synced).await?;
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
            synced.damaged += page.damaged;

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
        while !self.messages_page(folder_id, &mut synced).await? {}

        Ok(synced)
    }

    /// Fetch the next page of a folder's messages. True when that leaves the
    /// folder up to date, and there is no more to ask for now.
    async fn messages_page(&self, folder_id: &str, synced: &mut Synced) -> Result<bool, SyncError> {
        let state = self.db.messages_sync(folder_id)?;
        let page = match self
            .graph
            .messages_delta(folder_id, state.link.as_deref(), self.page_size)
            .await
        {
            Ok(page) => page,
            Err(GraphError::SyncStateGone) => {
                self.restarted(synced, "a folder")?;
                self.db.restart_messages_sync(folder_id)?;
                return Ok(false);
            }
            // The folder was deleted since the folder tree was read. Its
            // messages go when the next folder sync says it has gone.
            Err(GraphError::Refused { status: 404, .. }) => return Ok(true),
            Err(other) => return Err(other.into()),
        };

        let (changed, removed) = split(page.items, message_change);
        let round = round(&page.next);
        self.db
            .apply_messages(folder_id, &changed, &removed, page.next.url(), round)?;
        synced.pages += 1;
        synced.changed += count(&changed);
        synced.removed += count(&removed);
        synced.damaged += page.damaged;

        // As for folders: a full pass is followed at once by the round
        // that carries what changed while it ran.
        Ok(round == Round::Finished && !state.reconciling)
    }

    /// Learn which messages have been answered or forwarded, wherever that
    /// was done, and mark them. The server does not say so in a folder's
    /// sync, so it is asked on its own: the first time of the whole mailbox,
    /// and after that of what has changed since the last time.
    ///
    /// At most `pages` pages are fetched in one go. A mailbox with more than
    /// that carries on from where it stopped at the next sync. Gives back
    /// how many messages were marked.
    pub async fn sync_last_actions(&self, pages: u32) -> Result<u32, SyncError> {
        let mut link = self.db.setting(ACTIONS_LINK)?.filter(|l| !l.is_empty());
        let since = self.db.setting(ACTIONS_SINCE)?.filter(|s| !s.is_empty());
        // When this pass began, a little early: what changes while it runs,
        // or just before, is asked for again by the next.
        let began = match self.db.setting(ACTIONS_BEGAN)?.filter(|b| !b.is_empty()) {
            Some(began) if link.is_some() => began,
            _ => {
                let began = (jiff::Timestamp::now() - jiff::SignedDuration::from_mins(10))
                    .strftime("%Y-%m-%dT%H:%M:%SZ")
                    .to_string();
                self.db.set_setting(ACTIONS_BEGAN, &began)?;
                began
            }
        };

        let mut marked = 0;
        for _ in 0..pages {
            let page = self
                .graph
                .last_actions(link.as_deref(), since.as_deref())
                .await;
            let (found, next) = match page {
                Ok(page) => page,
                Err(error) => {
                    // A link the server no longer takes is dropped, and the
                    // pass begins again next time. Any other trouble is
                    // not the link's: with no connection, or the server
                    // busy, the place is kept, or a large mailbox on a
                    // poor connection would never reach the end of its
                    // first pass.
                    if matches!(
                        error,
                        GraphError::SyncStateGone
                            | GraphError::Refused {
                                status: 400 | 404 | 410,
                                ..
                            }
                    ) {
                        self.db.set_setting(ACTIONS_LINK, "")?;
                    }
                    return Err(error.into());
                }
            };
            for (id, verb) in found {
                let action = match verb {
                    102 => "replied",
                    103 => "replied_all",
                    104 => "forwarded",
                    _ => continue,
                };
                self.db.mark_last_action(&id, action)?;
                marked += 1;
            }
            match next {
                Some(next) => {
                    self.db.set_setting(ACTIONS_LINK, &next)?;
                    link = Some(next);
                }
                None => {
                    self.db.set_setting(ACTIONS_LINK, "")?;
                    self.db.set_setting(ACTIONS_SINCE, &began)?;
                    return Ok(marked);
                }
            }
        }

        Ok(marked)
    }

    /// Fetch the bodies of recent messages that are not here yet, newest
    /// first, so they can be read and searched with no connection. At most
    /// `limit` in one go: the rest wait for the next sync, which keeps any
    /// one sync short and well inside what Graph allows.
    ///
    /// `since` is a date, `YYYY-MM-DD`: messages received before it are left
    /// until they are opened.
    ///
    /// A message whose body cannot be had is passed over, so that it does
    /// not keep the ones after it from being fetched. Trouble that is the
    /// whole account's stops the fetching and is given back.
    pub async fn download_bodies(&self, since: &str, limit: u32) -> Result<u32, SyncError> {
        let mut fetched = 0;
        let mut passed_over = Outcome::default();

        for id in self.db.missing_bodies(since, limit)? {
            let body = match self.graph.message_body(&id).await {
                Ok(body) => Body {
                    content_type: body.content_type.to_ascii_lowercase(),
                    content: body.content,
                },
                // Deleted or moved out of the mailbox since it was listed.
                Err(GraphError::Refused { status: 404, .. }) => continue,
                Err(other) => {
                    if passed_over.fail(&id, other.into()) {
                        return passed_over.into_result().map(|_| fetched);
                    }
                    continue;
                }
            };
            passed_over.done(Synced::default());
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
    // Answered or sent on, where the server says. Anything else it may have
    // been last used for is not shown, and leaves what is known alone.
    let last_action = match message.last_verb() {
        Some(102) => Some("replied".to_owned()),
        Some(103) => Some("replied_all".to_owned()),
        Some(104) => Some("forwarded".to_owned()),
        _ => None,
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
        last_action,
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
        // Said only where the server says what kind of message it is.
        is_meeting: message
            .odata_type
            .as_deref()
            .map(|kind| kind.contains("eventMessage")),
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
