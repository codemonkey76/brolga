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
    /// When it is to be followed up by ("followupFlag resource type").
    pub due_date_time: Option<DateTimeTimeZone>,
}

/// A date and time with the zone it is in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DateTimeTimeZone {
    /// Without an offset: `2026-10-20T00:00:00.0000000`.
    pub date_time: String,
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemBody {
    /// `text` or `html`.
    pub content_type: String,
    pub content: String,
}

/// Something attached to a message, without its contents ("fileAttachment
/// resource type" and "Get attachment", SOURCES.md).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    /// `#microsoft.graph.fileAttachment`, `…itemAttachment` or `…referenceAttachment`.
    #[serde(rename = "@odata.type")]
    pub odata_type: Option<String>,
    /// What is shown for it, which need not be a file name.
    pub name: Option<String>,
    pub content_type: Option<String>,
    /// In bytes.
    pub size: Option<i64>,
    /// Shown within the body, as a picture in a signature is.
    pub is_inline: Option<bool>,
}

impl Attachment {
    /// `file`, `item` (a message, event or contact) or `reference` (a link
    /// to a file kept elsewhere, which has no contents to fetch).
    pub fn kind(&self) -> &'static str {
        let kind = self.odata_type.as_deref().unwrap_or_default();
        if kind.ends_with("itemAttachment") {
            "item"
        } else if kind.ends_with("referenceAttachment") {
            "reference"
        } else {
            "file"
        }
    }
}

/// A file attachment whole, contents included ("Get attachment", example 1).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileAttachment {
    pub id: String,
    pub content_type: Option<String>,
    /// What a body calls it by: `<img src="cid:…">`.
    pub content_id: Option<String>,
    /// The file, base64-encoded.
    pub content_bytes: Option<String>,
}

pub(crate) const ATTACHMENT_SELECT: &str = "id,name,contentType,size,isInline";

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
    pub bcc_recipients: Option<Vec<Recipient>>,
    pub received_date_time: Option<String>,
    pub sent_date_time: Option<String>,
    pub is_read: Option<bool>,
    pub is_draft: Option<bool>,
    /// What kind of message it is, where it is more than a plain one:
    /// `#microsoft.graph.eventMessageRequest` for a meeting invitation.
    #[serde(rename = "@odata.type")]
    pub odata_type: Option<String>,
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
