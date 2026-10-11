use serde::{Deserialize, Serialize};

/// The folders every mailbox has, by the names Graph knows them by
/// ("mailFolder resource type", SOURCES.md), in the order to show them.
pub const WELL_KNOWN_FOLDERS: &[&str] = &[
    "inbox",
    "drafts",
    "sentitems",
    "deleteditems",
    "archive",
    "junkemail",
    "outbox",
];

/// A mail folder. Every property but the id may be absent from a delta
/// entry, which carries only what changed.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailFolder {
    pub id: String,
    pub display_name: Option<String>,
    pub parent_folder_id: Option<String>,
    pub unread_item_count: Option<i64>,
    pub total_item_count: Option<i64>,
    pub is_hidden: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipient {
    #[serde(default)]
    pub email_address: EmailAddress,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Flag {
    /// `notFlagged`, `flagged` or `complete`.
    pub flag_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemBody {
    /// `text` or `html`.
    pub content_type: String,
    pub content: String,
}

/// A message, as far as the list needs it. The body is fetched on its own.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub parent_folder_id: Option<String>,
    pub conversation_id: Option<String>,
    pub subject: Option<String>,
    pub body_preview: Option<String>,
    pub from: Option<Recipient>,
    pub to_recipients: Option<Vec<Recipient>>,
    pub cc_recipients: Option<Vec<Recipient>>,
    pub received_date_time: Option<String>,
    pub sent_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub is_draft: Option<bool>,
    pub has_attachments: Option<bool>,
    /// `low`, `normal` or `high`.
    pub importance: Option<String>,
    pub flag: Option<Flag>,
    pub categories: Option<Vec<String>>,
    /// `focused` or `other`.
    pub inference_classification: Option<String>,
    pub body: Option<ItemBody>,
}

/// The message properties asked for in a sync: enough for the list, the
/// search index and the reading pane's header, and not the body.
pub(crate) const MESSAGE_SELECT: &str = "id,parentFolderId,conversationId,subject,bodyPreview,from,toRecipients,\
    ccRecipients,receivedDateTime,sentDateTime,isRead,isDraft,hasAttachments,importance,flag,categories,\
    inferenceClassification";

pub(crate) const FOLDER_SELECT: &str =
    "id,displayName,parentFolderId,unreadItemCount,totalItemCount,isHidden";
