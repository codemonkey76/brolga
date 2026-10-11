//! Mail brought in from an archive file.
//!
//! An archive is a mailbox with no server behind it: what is put here is
//! all there will ever be of it. So a message comes with its body and the
//! files attached to it, which mail from the server fetches only when asked.

use rusqlite::params;

use crate::account::{index, upsert_message};
use crate::{AccountDb, AttachmentRow, Body, FolderChange, InlineImage, MessageChange, StoreError};

/// A file attached to a message from an archive.
pub struct ArchivedAttachment {
    pub row: AttachmentRow,
    /// None where the archive had only its name: a message attached to a
    /// message, a link, or a file too large to keep.
    pub content: Option<Vec<u8>>,
}

/// One message from an archive, with everything of it there is.
pub struct ArchivedMessage {
    pub message: MessageChange,
    pub body: Option<Body>,
    /// The body as plain words, for search.
    pub text: String,
    pub attachments: Vec<ArchivedAttachment>,
    /// The pictures its body shows within itself.
    pub inline: Vec<InlineImage>,
}

impl AccountDb {
    /// Put an archive's folders in place, each under its parent.
    pub fn archive_folders(&self, folders: &[FolderChange]) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        for folder in folders {
            transaction.execute(
                "INSERT INTO folders (id, parent_id, name, unread, total, hidden, generation)
                 VALUES (?1, ?2, coalesce(?3, ''), 0, 0, 0, 0)
                 ON CONFLICT (id) DO UPDATE SET parent_id = ?2, name = coalesce(?3, name)",
                params![folder.id, folder.parent_id, folder.name],
            )?;
        }
        transaction.commit()?;

        Ok(())
    }

    /// Keep some of a folder's messages, all of them or none: a batch cut
    /// off part-way leaves nothing half-written.
    pub fn archive_messages(
        &self,
        folder_id: &str,
        messages: &[ArchivedMessage],
    ) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        for each in messages {
            let id = &each.message.id;
            upsert_message(&transaction, folder_id, &each.message, 0)?;
            if let Some(body) = &each.body {
                transaction.execute(
                    "INSERT OR REPLACE INTO bodies (message_id, content_type, content, text) VALUES (?1, ?2, ?3, ?4)",
                    params![id, body.content_type, body.content, each.text],
                )?;
            }
            index(&transaction, id)?;

            transaction.execute(
                "INSERT OR IGNORE INTO attachment_lists (message_id) VALUES (?1)",
                [id],
            )?;
            transaction.execute("DELETE FROM attachments WHERE message_id = ?1", [id])?;
            for (position, attached) in (0i64..).zip(&each.attachments) {
                let row = &attached.row;
                transaction.execute(
                    "INSERT INTO attachments (message_id, position, id, name, content_type, size, is_inline, kind)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![id, position, row.id, row.name, row.content_type, row.size, row.is_inline, row.kind],
                )?;
                if let Some(content) = &attached.content {
                    transaction.execute(
                        "INSERT OR REPLACE INTO attachment_content (message_id, attachment_id, content) VALUES (?1, ?2, ?3)",
                        params![id, row.id, content],
                    )?;
                }
            }

            transaction.execute(
                "INSERT OR IGNORE INTO inline_image_sets (message_id) VALUES (?1)",
                [id],
            )?;
            for image in &each.inline {
                transaction.execute(
                    "INSERT OR IGNORE INTO inline_images (message_id, content_id, content_type, data) VALUES (?1, ?2, ?3, ?4)",
                    params![id, image.content_id, image.content_type, image.data],
                )?;
            }
        }
        transaction.commit()?;

        Ok(())
    }

    /// Count each folder's messages, and those unread, from what is here.
    /// The server says these for its own mail; for an archive nobody else can.
    pub fn count_folders(&self) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE folders SET
                 total = (SELECT count(*) FROM messages m WHERE m.folder_id = folders.id),
                 unread = (SELECT count(*) FROM messages m WHERE m.folder_id = folders.id AND m.is_read = 0)",
            [],
        )?;

        Ok(())
    }

    /// A file attached to a message from an archive, if it was kept.
    pub fn attachment_content(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        use rusqlite::OptionalExtension;

        Ok(self
            .connection()
            .query_row(
                "SELECT content FROM attachment_content WHERE message_id = ?1 AND attachment_id = ?2",
                params![message_id, attachment_id],
                |row| row.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, read: bool) -> ArchivedMessage {
        ArchivedMessage {
            message: MessageChange {
                id: id.to_owned(),
                subject: Some(format!("Subject of {id}")),
                from_name: Some("Sam Taylor".to_owned()),
                from_address: Some("sam@example.test".to_owned()),
                received_at: Some("2024-11-03T00:00:00Z".to_owned()),
                is_read: Some(read),
                has_attachments: Some(true),
                ..MessageChange::default()
            },
            body: Some(Body {
                content_type: "html".to_owned(),
                content: "<p>The harbour plan</p>".to_owned(),
            }),
            text: "The harbour plan".to_owned(),
            attachments: vec![
                ArchivedAttachment {
                    row: AttachmentRow {
                        id: "1".to_owned(),
                        name: "notes.txt".to_owned(),
                        content_type: "text/plain".to_owned(),
                        size: 11,
                        is_inline: false,
                        kind: "file".to_owned(),
                    },
                    content: Some(b"Some notes.".to_vec()),
                },
                ArchivedAttachment {
                    row: AttachmentRow {
                        id: "2".to_owned(),
                        name: "Forwarded".to_owned(),
                        content_type: String::new(),
                        size: 0,
                        is_inline: false,
                        kind: "item".to_owned(),
                    },
                    content: None,
                },
            ],
            inline: vec![InlineImage {
                content_id: "plan@example.test".to_owned(),
                content_type: "image/png".to_owned(),
                data: "AAAA".to_owned(),
            }],
        }
    }

    fn folder(id: &str, parent: Option<&str>, name: &str) -> FolderChange {
        FolderChange {
            id: id.to_owned(),
            parent_id: parent.map(str::to_owned),
            name: Some(name.to_owned()),
            unread: None,
            total: None,
            hidden: None,
        }
    }

    #[test]
    fn an_archived_message_is_here_whole_and_can_be_searched() {
        let db = AccountDb::in_memory().unwrap();
        db.archive_folders(&[
            folder("f1", None, "Inbox"),
            folder("f2", Some("f1"), "Harbour"),
        ])
        .unwrap();
        db.archive_messages("f2", &[message("m1", false), message("m2", true)])
            .unwrap();
        db.count_folders().unwrap();

        let folders = db.folders().unwrap();
        let harbour = folders.iter().find(|each| each.name == "Harbour").unwrap();
        assert_eq!((harbour.total, harbour.unread, harbour.depth), (2, 1, 1));
        assert_eq!(
            db.body("m1").unwrap().unwrap().content,
            "<p>The harbour plan</p>"
        );
        let attached = db.attachments("m1").unwrap().unwrap();
        assert_eq!(attached.len(), 2);
        assert_eq!(
            db.attachment_content("m1", "1").unwrap().as_deref(),
            Some(&b"Some notes."[..])
        );
        assert_eq!(db.attachment_content("m1", "2").unwrap(), None);
        assert_eq!(db.inline_images("m1").unwrap().unwrap().len(), 1);
        let found = db
            .search("harbour", None, None, None, None, None, 10)
            .unwrap();
        assert_eq!(found.len(), 2);

        // Brought in again, nothing is there twice.
        db.archive_messages("f2", &[message("m1", false)]).unwrap();
        assert_eq!(db.message_count("f2").unwrap(), 2);
        assert_eq!(db.attachments("m1").unwrap().unwrap().len(), 2);
        // And a message that goes takes its files with it.
        db.forget_message("m1").unwrap();
        assert_eq!(db.attachment_content("m1", "1").unwrap(), None);
    }
}
