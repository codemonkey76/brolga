//! Reading PST files: the folders in one, and each message whole.
//!
//! The format is Microsoft's published one (MS-PST, SOURCES.md), read by
//! their own open-source parser. The file is opened for reading and nothing
//! else, so nothing here can change it. It is never read into memory whole:
//! a folder's list of messages is, and then one message at a time.
//!
//! A PST is whatever someone handed over, so nothing in one is trusted: a
//! part that cannot be read is an error for that folder or message alone,
//! and the rest can still be read.

use std::fs::File;
use std::io;
use std::path::Path;
use std::rc::Rc;

use outlook_pst::ltp::prop_context::PropertyValue;
use outlook_pst::ltp::table_context::TableContext;
use outlook_pst::messaging::attachment::{
    AnsiAttachment, Attachment as _, AttachmentData, UnicodeAttachment,
};
use outlook_pst::messaging::message::{AnsiMessage, Message as _, UnicodeMessage};
use outlook_pst::messaging::store::{AnsiStore, Store, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use outlook_pst::{AnsiPstFile, UnicodePstFile};

/// The largest attachment kept. A larger one is named, with its size, and
/// its content left behind.
pub const LARGEST_ATTACHMENT: usize = 512 * 1024 * 1024;

/// The deepest a folder may sit. A file whose folders go deeper, or round
/// in a ring, is not followed further.
const DEEPEST: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum PstError {
    #[error("the file could not be opened: {0}")]
    Open(io::Error),
    #[error("this is not a PST or OST file, or it is damaged: {0}")]
    NotPst(io::Error),
    #[error("part of the file could not be read: {0}")]
    Read(#[from] io::Error),
}

/// A folder in the file. Ids are the file's own and mean nothing outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: u32,
    /// None for a folder at the top.
    pub parent: Option<u32>,
    pub name: String,
    /// The messages in it, in the order the file lists them.
    pub messages: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub address: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub content_type: String,
    /// What the message's HTML calls it by, for a picture shown within it.
    pub content_id: String,
    pub is_inline: bool,
    /// In bytes, as the file says, whether or not the content was kept.
    pub size: u64,
    /// The file itself. None for a message attached to a message, a link
    /// to a file kept elsewhere, and anything over [`LARGEST_ATTACHMENT`].
    pub content: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    /// `IPM.Note` for mail, `IPM.Appointment` for a calendar entry, and so on.
    pub class: String,
    pub subject: String,
    pub from: Person,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub bcc: Vec<Person>,
    /// UTC, as `2026-10-20T03:00:00Z`. None where the file does not say.
    pub received_at: Option<String>,
    pub sent_at: Option<String>,
    pub is_read: bool,
    pub is_draft: bool,
    /// `low`, `normal` or `high`.
    pub importance: String,
    /// `notFlagged`, `flagged` or `complete`.
    pub flag_status: String,
    pub internet_message_id: String,
    /// What the conversation is called, which messages of one thread share.
    pub conversation_topic: String,
    pub categories: Vec<String>,
    pub html: Option<String>,
    pub text: Option<String>,
    pub attachments: Vec<Attachment>,
    /// For a calendar entry: when it starts and ends (UTC) and where.
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub location: String,
}

enum Opened {
    Unicode(Rc<UnicodeStore>),
    Ansi(Rc<AnsiStore>),
}

/// A PST or OST file, open for reading.
pub struct Pst {
    opened: Opened,
}

/// A time as the file keeps it, in 100-nanosecond steps from 1601, as UTC.
fn moment(value: Option<&PropertyValue>) -> Option<String> {
    let Some(PropertyValue::Time(steps)) = value else {
        return None;
    };
    let seconds = steps.checked_div(10_000_000)?.checked_sub(11_644_473_600)?;
    // A time before 1970 or far ahead is a blank or a mistake, not a date.
    if !(0..=32_503_680_000).contains(&seconds) {
        return None;
    }
    let at = jiff::Timestamp::from_second(seconds).ok()?;

    Some(at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
}

fn words(value: Option<&PropertyValue>) -> String {
    match value {
        Some(PropertyValue::Unicode(value)) => value.to_string(),
        // The older files' one-byte text, read as Latin-1.
        Some(PropertyValue::String8(value)) => value.to_string(),
        _ => String::new(),
    }
    .trim_matches('\0')
    .to_owned()
}

fn number(value: Option<&PropertyValue>) -> Option<i64> {
    match value {
        Some(PropertyValue::Integer32(n)) => Some(i64::from(*n)),
        Some(PropertyValue::Integer16(n)) => Some(i64::from(*n)),
        Some(PropertyValue::Integer64(n)) => Some(*n),
        _ => None,
    }
}

fn flag(value: Option<&PropertyValue>) -> bool {
    matches!(value, Some(PropertyValue::Boolean(true)))
}

/// A subject as written, without the two marks some files put before it
/// to say how long its prefix is.
fn subject(value: Option<&PropertyValue>) -> String {
    let subject = words(value);
    let mut marks = subject.chars();
    match marks.next() {
        Some('\u{1}') => marks.skip(1).collect(),
        _ => subject,
    }
}

/// Bytes in a Windows code page, as text. One not known is read as
/// Windows-1252, which is what most mail in such files was written in.
fn decoded(bytes: &[u8], code_page: Option<i64>) -> String {
    let label = match code_page {
        Some(65001) | None => "utf-8".to_owned(),
        Some(20127) => "windows-1252".to_owned(),
        Some(28591) => "iso-8859-1".to_owned(),
        Some(page @ 28592..=28606) => format!("iso-8859-{}", page - 28590),
        Some(932) => "shift_jis".to_owned(),
        Some(936) => "gbk".to_owned(),
        Some(949) => "euc-kr".to_owned(),
        Some(950) => "big5".to_owned(),
        Some(50220) => "iso-2022-jp".to_owned(),
        Some(51932) => "euc-jp".to_owned(),
        Some(20866) => "koi8-r".to_owned(),
        Some(page) => format!("windows-{page}"),
    };
    let encoding =
        encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252);

    encoding.decode(bytes).0.into_owned()
}

/// An address as mail is sent to one. The address of someone inside an
/// Exchange organisation is kept in another form, which is no use outside
/// it, so their SMTP address is preferred wherever the file has it.
fn address(
    smtp: Option<&PropertyValue>,
    kind: Option<&PropertyValue>,
    other: Option<&PropertyValue>,
) -> String {
    let smtp = words(smtp);
    if smtp.contains('@') {
        return smtp;
    }
    let other = words(other);
    if words(kind).eq_ignore_ascii_case("EX") && !other.contains('@') {
        return String::new();
    }

    other
}

/// Every row of a table as its named columns.
fn rows(table: &dyn TableContext, wanted: &[u16]) -> Vec<(u32, Vec<(u16, PropertyValue)>)> {
    let context = table.context();
    table
        .rows_matrix()
        .map(|row| {
            let values = row
                .columns(context)
                .map(|values| {
                    context
                        .columns()
                        .iter()
                        .zip(values)
                        .filter(|(column, _)| wanted.contains(&column.prop_id()))
                        .filter_map(|(column, value)| {
                            let value = table.read_column(&value?, column.prop_type()).ok()?;
                            Some((column.prop_id(), value))
                        })
                        .collect()
                })
                .unwrap_or_default();
            (u32::from(row.id()), values)
        })
        .collect()
}

fn people(table: Option<&Rc<dyn TableContext>>) -> (Vec<Person>, Vec<Person>, Vec<Person>) {
    let (mut to, mut cc, mut bcc) = (Vec::new(), Vec::new(), Vec::new());
    let Some(table) = table else {
        return (to, cc, bcc);
    };
    for (_, row) in rows(table.as_ref(), &[0x0C15, 0x3001, 0x3002, 0x3003, 0x39FE]) {
        let get = |id: u16| {
            row.iter()
                .find(|(each, _)| *each == id)
                .map(|(_, value)| value)
        };
        let person = Person {
            name: words(get(0x3001)),
            address: address(get(0x39FE), get(0x3002), get(0x3003)),
        };
        if person.name.is_empty() && person.address.is_empty() {
            continue;
        }
        match number(get(0x0C15)) {
            Some(2) => cc.push(person),
            Some(3) => bcc.push(person),
            _ => to.push(person),
        }
    }

    (to, cc, bcc)
}

/// One attachment, from its properties and whatever content came with them.
fn attachment(
    get: &dyn Fn(u16) -> Option<PropertyValue>,
    data: Option<&AttachmentData>,
) -> Attachment {
    let name = [0x3707, 0x3704, 0x3001]
        .iter()
        .map(|id| words(get(*id).as_ref()))
        .find(|name| !name.is_empty())
        .unwrap_or_default();
    let content_id = words(get(0x3712).as_ref());
    let content = match data {
        Some(AttachmentData::Binary(bytes)) if bytes.buffer().len() <= LARGEST_ATTACHMENT => {
            Some(bytes.buffer().to_vec())
        }
        _ => None,
    };
    let size = content
        .as_ref()
        .map(|content| content.len() as u64)
        .or_else(|| number(get(0x0E20).as_ref()).and_then(|size| u64::try_from(size).ok()))
        .unwrap_or_default();

    Attachment {
        name,
        content_type: words(get(0x370E).as_ref()),
        // Hidden from the list, or named by the HTML: a picture within the message.
        is_inline: flag(get(0x7FFE).as_ref()) || !content_id.is_empty(),
        content_id,
        size,
        content,
    }
}

/// A message from its properties, with nobody on it and nothing attached yet.
fn message(get: &dyn Fn(u16) -> Option<PropertyValue>) -> Message {
    let words_of = |id: u16| words(get(id).as_ref());
    let flags = number(get(0x0E07).as_ref()).unwrap_or_default();
    let code_page = number(get(0x3FDE).as_ref());
    let html = match get(0x1013) {
        Some(PropertyValue::Binary(bytes)) => Some(decoded(bytes.buffer(), code_page)),
        Some(other @ (PropertyValue::Unicode(_) | PropertyValue::String8(_))) => {
            Some(words(Some(&other)))
        }
        _ => None,
    }
    .filter(|html| !html.trim().is_empty());
    let text = Some(words_of(0x1000)).filter(|text| !text.trim().is_empty());
    // Who it is from: who it was sent for, where someone sent it for another.
    let from = Person {
        name: [0x0042, 0x0C1A]
            .iter()
            .map(|id| words_of(*id))
            .find(|name| !name.is_empty())
            .unwrap_or_default(),
        address: [(0x5D02, 0x0064, 0x0065), (0x5D01, 0x0C1E, 0x0C1F)]
            .iter()
            .map(|(smtp, kind, other)| {
                address(
                    get(*smtp).as_ref(),
                    get(*kind).as_ref(),
                    get(*other).as_ref(),
                )
            })
            .find(|address| !address.is_empty())
            .unwrap_or_default(),
    };
    Message {
        class: words_of(0x001A),
        subject: subject(get(0x0037).as_ref()),
        from,
        received_at: moment(get(0x0E06).as_ref()),
        sent_at: moment(get(0x0039).as_ref()),
        is_read: flags & 0x1 != 0,
        is_draft: flags & 0x8 != 0,
        importance: match number(get(0x0017).as_ref()) {
            Some(0) => "low",
            Some(2) => "high",
            _ => "normal",
        }
        .to_owned(),
        flag_status: match number(get(0x1090).as_ref()) {
            Some(1) => "complete",
            Some(2) => "flagged",
            _ => "notFlagged",
        }
        .to_owned(),
        internet_message_id: words_of(0x1035),
        conversation_topic: words_of(0x0070),
        html,
        text,
        ..Message::default()
    }
}

macro_rules! read_message {
    ($store:expr, $message:ty, $attachment:ty, $id:expr) => {{
        let store = $store;
        let entry = store.properties().make_entry_id(NodeId::from($id))?;
        let opened = <$message>::read(store.clone(), &entry, None)?;
        let properties = opened.properties();
        let mut message = message(&|id| properties.get(id).cloned());
        (message.to, message.cc, message.bcc) = people(opened.recipient_table());
        if let Some(table) = opened.attachment_table() {
            let listed: Vec<u32> = table.rows_matrix().map(|row| u32::from(row.id())).collect();
            for row in listed {
                // One that cannot be read is left out; the message is still a message.
                let Ok(attached) = <$attachment>::read(opened.clone(), NodeId::from(row), None)
                else {
                    continue;
                };
                let properties = attached.properties();
                message.attachments.push(attachment(
                    &|id| properties.get(id).cloned(),
                    attached.data(),
                ));
            }
        }

        Ok(message)
    }};
}

impl Pst {
    /// Open a file to read. It is opened for reading alone: nothing done
    /// through this can write to it.
    pub fn open(path: &Path) -> Result<Self, PstError> {
        let file = || File::open(path).map_err(PstError::Open);
        let opened = match UnicodePstFile::read_from(Box::new(file()?)) {
            Ok(unicode) => {
                Opened::Unicode(UnicodeStore::read(Rc::new(unicode)).map_err(PstError::NotPst)?)
            }
            // Not the newer form: the older one, or not a PST at all.
            Err(_) => {
                let ansi = AnsiPstFile::read_from(Box::new(file()?)).map_err(PstError::NotPst)?;
                Opened::Ansi(AnsiStore::read(Rc::new(ansi)).map_err(PstError::NotPst)?)
            }
        };

        Ok(Self { opened })
    }

    fn store(&self) -> Rc<dyn Store> {
        match &self.opened {
            Opened::Unicode(store) => store.clone(),
            Opened::Ansi(store) => store.clone(),
        }
    }

    /// Every folder a person would see, parents before their children. A
    /// folder that cannot be read is left out with all beneath it, and
    /// counted in the second number.
    pub fn folders(&self) -> Result<(Vec<Folder>, usize), PstError> {
        let store = self.store();
        let top = store.properties().ipm_sub_tree_entry_id()?;
        let top = store.open_folder(&top)?;
        let children = |folder: &dyn outlook_pst::messaging::folder::Folder| -> Vec<u32> {
            folder
                .hierarchy_table()
                .map(|table| table.rows_matrix().map(|row| u32::from(row.id())).collect())
                .unwrap_or_default()
        };

        let (mut folders, mut unread) = (Vec::new(), 0);
        let mut seen = std::collections::HashSet::new();
        // Each to come: its id, its parent, and how deep it is.
        let mut waiting: Vec<(u32, Option<u32>, usize)> = children(top.as_ref())
            .into_iter()
            .rev()
            .map(|id| (id, None, 0))
            .collect();
        while let Some((id, parent, depth)) = waiting.pop() {
            if depth > DEEPEST || !seen.insert(id) {
                continue;
            }
            let opened = store
                .properties()
                .make_entry_id(NodeId::from(id))
                .and_then(|entry| store.open_folder(&entry));
            let Ok(folder) = opened else {
                unread += 1;
                continue;
            };
            let messages = folder
                .contents_table()
                .map(|table| table.rows_matrix().map(|row| u32::from(row.id())).collect())
                .unwrap_or_default();
            folders.push(Folder {
                id,
                parent,
                name: folder.properties().display_name().unwrap_or_default(),
                messages,
            });
            waiting.extend(
                children(folder.as_ref())
                    .into_iter()
                    .rev()
                    .map(|child| (child, Some(id), depth + 1)),
            );
        }

        Ok((folders, unread))
    }

    /// One message whole, by its id in a folder's list.
    pub fn message(&self, id: u32) -> Result<Message, PstError> {
        match &self.opened {
            Opened::Unicode(store) => read_message!(store, UnicodeMessage, UnicodeAttachment, id),
            Opened::Ansi(store) => read_message!(store, AnsiMessage, AnsiAttachment, id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pst_writer::{
        UnicodePstAttachment, UnicodePstFile, UnicodePstMessage, UnicodePstRecipient,
        UnicodePstRecipientType,
    };

    /// A file of this test's own, gone when the test is.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let folder =
                std::env::temp_dir().join(format!("brolga-pst-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&folder).unwrap();
            Self(folder.join("mail.pst"))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            if let Some(folder) = self.0.parent() {
                let _ = std::fs::remove_dir_all(folder);
            }
        }
    }

    const PICTURE: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3, 4];

    fn sample(path: &Path) {
        let recipients = [
            UnicodePstRecipient {
                name: "Robin Example",
                email: "robin@example.test",
                recipient_type: UnicodePstRecipientType::To,
            },
            UnicodePstRecipient {
                name: "Jo Nguyen",
                email: "jo@example.test",
                recipient_type: UnicodePstRecipientType::Cc,
            },
            UnicodePstRecipient {
                name: "Hidden",
                email: "hidden@example.test",
                recipient_type: UnicodePstRecipientType::Bcc,
            },
        ];
        let first = UnicodePstMessage {
            subject: "Budget review — moved",
            sender_name: "Sam Taylor",
            sender_email: "sam@example.test",
            recipients: &recipients,
            body: "The room is booked for Thursday.",
            html_body: Some(
                "<p>The room is booked for <b>Thursday</b>.</p><img src=\"cid:plan@example.test\">",
            ),
            message_id: "<one@example.test>",
            // 2024-11-03T00:00:00Z.
            delivery_time: 133_750_656_000_000_000,
        };
        let attachments = [
            UnicodePstAttachment {
                filename: "plan.png",
                mime_type: "image/png",
                content_id: Some("<plan@example.test>"),
                data: PICTURE,
            },
            UnicodePstAttachment {
                filename: "notes.txt",
                mime_type: "text/plain",
                content_id: None,
                data: b"Some notes.",
            },
        ];
        drop(UnicodePstFile::create_with_attachments(path, &first, &attachments).unwrap());
        drop(
            UnicodePstFile::append(
                path,
                &UnicodePstMessage {
                    subject: "Plain words only",
                    html_body: None,
                    message_id: "<two@example.test>",
                    ..first
                },
            )
            .unwrap(),
        );
        drop(
            UnicodePstFile::append_in_folder(
                path,
                &["Projects", "Harbour"],
                &UnicodePstMessage {
                    subject: "In a folder of its own",
                    message_id: "<three@example.test>",
                    ..first
                },
            )
            .unwrap(),
        );
    }

    #[test]
    fn the_folders_come_parents_first_each_with_its_messages() {
        let file = Scratch::new("folders");
        sample(&file.0);

        let (folders, unread) = Pst::open(&file.0).unwrap().folders().unwrap();

        assert_eq!(unread, 0);
        let named = |name: &str| folders.iter().find(|folder| folder.name == name).unwrap();
        assert_eq!(named("Inbox").messages.len(), 2);
        assert_eq!(named("Inbox").parent, None);
        assert_eq!(named("Harbour").parent, Some(named("Projects").id));
        assert_eq!(named("Harbour").messages.len(), 1);
        let place = |name: &str| {
            folders
                .iter()
                .position(|folder| folder.name == name)
                .unwrap()
        };
        assert!(place("Projects") < place("Harbour"));
    }

    #[test]
    fn a_message_is_read_whole_with_who_it_went_to_and_what_was_attached() {
        let file = Scratch::new("message");
        sample(&file.0);
        let pst = Pst::open(&file.0).unwrap();
        let (folders, _) = pst.folders().unwrap();
        let inbox = folders
            .iter()
            .find(|folder| folder.name == "Inbox")
            .unwrap();
        let messages: Vec<Message> = inbox
            .messages
            .iter()
            .map(|id| pst.message(*id).unwrap())
            .collect();
        let message = messages
            .iter()
            .find(|each| each.subject.starts_with("Budget"))
            .unwrap();

        assert_eq!(message.subject, "Budget review — moved");
        assert_eq!(message.class, "IPM.Note");
        assert_eq!(
            message.from,
            Person {
                name: "Sam Taylor".to_owned(),
                address: "sam@example.test".to_owned()
            }
        );
        assert_eq!(
            message.to,
            [Person {
                name: "Robin Example".to_owned(),
                address: "robin@example.test".to_owned()
            }]
        );
        assert_eq!(message.cc[0].address, "jo@example.test");
        assert_eq!(message.bcc[0].address, "hidden@example.test");
        assert_eq!(message.received_at.as_deref(), Some("2024-11-03T00:00:00Z"));
        assert_eq!(message.internet_message_id, "<one@example.test>");
        assert!(message.html.as_deref().unwrap().contains("<b>Thursday</b>"));
        assert_eq!(
            message.text.as_deref(),
            Some("The room is booked for Thursday.")
        );
        assert_eq!(message.importance, "normal");

        let picture = message
            .attachments
            .iter()
            .find(|each| each.name == "plan.png")
            .unwrap();
        assert!(picture.is_inline);
        assert_eq!(picture.content_id, "plan@example.test");
        assert_eq!(picture.content.as_deref(), Some(PICTURE));
        let notes = message
            .attachments
            .iter()
            .find(|each| each.name == "notes.txt")
            .unwrap();
        assert!(!notes.is_inline);
        assert_eq!(notes.content_type, "text/plain");
        assert_eq!(notes.content.as_deref(), Some(&b"Some notes."[..]));
        assert_eq!(notes.size, 11);

        let plain = messages
            .iter()
            .find(|each| each.subject == "Plain words only")
            .unwrap();
        assert_eq!(plain.html, None);
        assert!(plain.attachments.is_empty());
    }

    #[test]
    fn reading_a_file_leaves_it_exactly_as_it_was() {
        let file = Scratch::new("untouched");
        sample(&file.0);
        let before = std::fs::read(&file.0).unwrap();
        let changed = std::fs::metadata(&file.0).unwrap().modified().unwrap();

        let pst = Pst::open(&file.0).unwrap();
        for folder in pst.folders().unwrap().0 {
            for id in folder.messages {
                pst.message(id).unwrap();
            }
        }
        drop(pst);

        assert_eq!(std::fs::read(&file.0).unwrap(), before);
        assert_eq!(
            std::fs::metadata(&file.0).unwrap().modified().unwrap(),
            changed
        );
    }

    #[test]
    fn a_file_that_cannot_be_written_to_is_read_all_the_same() {
        let file = Scratch::new("read-only");
        sample(&file.0);
        let mut permissions = std::fs::metadata(&file.0).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&file.0, permissions.clone()).unwrap();

        let folders = Pst::open(&file.0).unwrap().folders().unwrap().0;
        assert!(folders.iter().any(|folder| folder.name == "Inbox"));

        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&file.0, permissions).unwrap();
    }

    #[test]
    fn what_is_not_a_pst_says_so_and_a_missing_file_says_that() {
        let file = Scratch::new("not-pst");
        std::fs::write(&file.0, b"This is a letter, not a mailbox.").unwrap();
        assert!(matches!(Pst::open(&file.0), Err(PstError::NotPst(_))));
        // The start of a real one, cut off.
        let whole = Scratch::new("cut-off");
        sample(&whole.0);
        let bytes = std::fs::read(&whole.0).unwrap();
        std::fs::write(&file.0, &bytes[..600]).unwrap();
        assert!(Pst::open(&file.0).is_err());

        assert!(matches!(
            Pst::open(Path::new("/no/such/file.pst")),
            Err(PstError::Open(_))
        ));
    }

    #[test]
    fn times_and_subjects_are_read_as_the_file_keeps_them() {
        assert_eq!(
            moment(Some(&PropertyValue::Time(133_750_656_000_000_000))).as_deref(),
            Some("2024-11-03T00:00:00Z")
        );
        // Blank, and before there was mail.
        assert_eq!(moment(Some(&PropertyValue::Time(0))), None);
        assert_eq!(moment(None), None);
        assert_eq!(decoded(b"caf\xe9", Some(1252)), "café");
        assert_eq!(decoded("café".as_bytes(), Some(65001)), "café");
        assert_eq!(decoded(b"caf\xe9", Some(99_999)), "café");
    }
}
