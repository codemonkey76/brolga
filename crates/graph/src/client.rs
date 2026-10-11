use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use reqwest::header::{AUTHORIZATION, RETRY_AFTER, WWW_AUTHENTICATE};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;

use crate::delta::{read_list_page, read_page};
use crate::mail::{ATTACHMENT_SELECT, FOLDER_SELECT, MESSAGE_SELECT};
use crate::{
    Attachment, Calendar, DeltaPage, Event, FileAttachment, GraphError, ItemBody, MailFolder,
    Message, RetryPolicy,
};

/// How many requests may be in flight for one mailbox at once. Low on
/// purpose: Graph throttles a mailbox on concurrency as well as on volume.
const IN_FLIGHT: usize = 4;

/// Supplies an access token for the account a client is for.
pub trait TokenSource: Send + Sync {
    fn token(&self) -> BoxFuture<'_, Result<String, GraphError>>;
}

/// A Graph client for one mailbox: the signed-in person's own (`/me`).
pub struct GraphClient {
    http: reqwest::Client,
    /// `https://graph.microsoft.com/v1.0`, or a test server's stand-in for it.
    base: String,
    tokens: Arc<dyn TokenSource>,
    retry: RetryPolicy,
    in_flight: Semaphore,
    /// Whose mailbox requests are for, where it is not the mailbox of
    /// whoever is signed in: another's, opened by its address.
    mailbox: Option<String>,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

impl GraphClient {
    pub fn new(
        base: impl Into<String>,
        tokens: Arc<dyn TokenSource>,
        retry: RetryPolicy,
    ) -> Result<Self, GraphError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(GraphError::Network)?;

        Ok(Self {
            http,
            base: base.into().trim_end_matches('/').to_owned(),
            tokens,
            retry,
            in_flight: Semaphore::new(IN_FLIGHT),
            mailbox: None,
        })
    }

    /// The same client for a mailbox someone else has given access to: a
    /// shared mailbox, by its address. Every request written for `/me` is
    /// then made of `/users/{address}` ("Get message", SOURCES.md).
    pub fn for_mailbox(mut self, address: &str) -> Self {
        self.mailbox = Some(address.to_owned());
        self
    }

    /// A request's address, for the mailbox this client is for.
    fn addressed<'a>(&self, url: &'a str) -> std::borrow::Cow<'a, str> {
        let own = format!("{}/me", self.base);
        match (&self.mailbox, url.strip_prefix(&own)) {
            // `/me` itself, `/me/…` or `/me?…`: not a longer name that begins the same.
            (Some(address), Some(rest)) if rest.is_empty() || rest.starts_with(['/', '?']) => {
                format!("{}/users/{address}{rest}", self.base).into()
            }
            _ => url.into(),
        }
    }

    pub fn production(tokens: Arc<dyn TokenSource>) -> Result<Self, GraphError> {
        Self::new(
            "https://graph.microsoft.com/v1.0",
            tokens,
            RetryPolicy::default(),
        )
    }

    /// One page of changes to the folder tree. `link` is where the last page
    /// said to go; without one, the sync starts from the beginning.
    pub async fn mail_folders_delta(
        &self,
        link: Option<&str>,
    ) -> Result<DeltaPage<MailFolder>, GraphError> {
        let start = format!("{}/me/mailFolders/delta?$select={FOLDER_SELECT}", self.base);

        read_page(&self.get(link.unwrap_or(&start), Some(50)).await?)
    }

    /// One page of changes to the messages in a folder, newest first on a
    /// first sync so that a large folder is usable before it is complete.
    pub async fn messages_delta(
        &self,
        folder_id: &str,
        link: Option<&str>,
        page_size: u32,
    ) -> Result<DeltaPage<Message>, GraphError> {
        let start = format!(
            "{}/me/mailFolders/{folder_id}/messages/delta?$select={MESSAGE_SELECT}&$orderby=receivedDateTime+desc",
            self.base
        );

        read_page(&self.get(link.unwrap_or(&start), Some(page_size)).await?)
    }

    /// Every calendar the mailbox has ("List calendars", SOURCES.md).
    pub async fn calendars(&self) -> Result<Vec<Calendar>, GraphError> {
        let mut url = format!("{}/me/calendars", self.base);
        let mut all = Vec::new();
        loop {
            let page: DeltaPage<Calendar> = read_list_page(&self.get(&url, Some(100)).await?)?;
            for item in page.items {
                if let crate::DeltaItem::Changed(calendar) = item {
                    all.push(calendar);
                }
            }
            match page.next {
                crate::Next::Page(next) => url = next,
                crate::Next::Done(_) => return Ok(all),
            }
        }
    }

    /// One page of changes to the events of the default calendar between
    /// two moments ("event: delta"). Each occurrence of a repeating event is
    /// an event of its own. `link` is where the last page said to go;
    /// without one the sync starts from the beginning, over `window`: two
    /// moments in UTC, written as `2026-01-01T00:00:00`.
    pub async fn events_delta(
        &self,
        link: Option<&str>,
        window: (&str, &str),
        page_size: u32,
    ) -> Result<DeltaPage<Event>, GraphError> {
        let start = format!(
            "{}/me/calendarView/delta?startDateTime={}&endDateTime={}",
            self.base, window.0, window.1
        );

        read_page(&self.get(link.unwrap_or(&start), Some(page_size)).await?)
    }

    /// One page of the events of a calendar between two moments ("List
    /// calendarView"). Not a delta: Graph has one only for the default
    /// calendar, so any other is read whole.
    pub async fn calendar_view(
        &self,
        calendar_id: &str,
        link: Option<&str>,
        window: (&str, &str),
        page_size: u32,
    ) -> Result<DeltaPage<Event>, GraphError> {
        let start = format!(
            "{}/me/calendars/{calendar_id}/calendarView?startDateTime={}&endDateTime={}",
            self.base, window.0, window.1
        );

        read_list_page(&self.get(link.unwrap_or(&start), Some(page_size)).await?)
    }

    /// One event whole, with its body and, for a series master, how it repeats.
    pub async fn event(&self, event_id: &str) -> Result<Event, GraphError> {
        let url = format!("{}/me/events/{event_id}", self.base);

        decode(&self.get(&url, None).await?)
    }

    /// Make a calendar ("Create calendar", SOURCES.md). Not safe to repeat
    /// blindly: a second request makes a second calendar of the same name.
    pub async fn create_calendar(&self, name: &str) -> Result<Calendar, GraphError> {
        let url = format!("{}/me/calendars", self.base);
        let body = serde_json::json!({ "name": name });

        decode(
            &self
                .send(reqwest::Method::POST, &url, None, Some(&body), false)
                .await?,
        )
    }

    /// Delete a calendar and everything in it ("Delete calendar",
    /// SOURCES.md). One that is already gone is as good as deleted.
    pub async fn delete_calendar(&self, calendar_id: &str) -> Result<(), GraphError> {
        let url = format!("{}/me/calendars/{calendar_id}", self.base);

        match self
            .send(reqwest::Method::DELETE, &url, None, None, true)
            .await
        {
            Ok(_) | Err(GraphError::Refused { status: 404, .. }) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// Have done with an event's reminder, for every device ("event:
    /// dismissReminder", SOURCES.md).
    pub async fn dismiss_reminder(&self, event_id: &str) -> Result<(), GraphError> {
        let url = format!("{}/me/events/{event_id}/dismissReminder", self.base);
        self.send(reqwest::Method::POST, &url, None, None, true)
            .await?;

        Ok(())
    }

    /// Put an event's reminder off until a moment in UTC, for every device
    /// ("event: snoozeReminder", SOURCES.md).
    pub async fn snooze_reminder(&self, event_id: &str, until: &str) -> Result<(), GraphError> {
        let url = format!("{}/me/events/{event_id}/snoozeReminder", self.base);
        let body = serde_json::json!({
            "newReminderTime": { "dateTime": until.trim_end_matches('Z'), "timeZone": "UTC" }
        });
        self.send(reqwest::Method::POST, &url, None, Some(&body), true)
            .await?;

        Ok(())
    }

    /// Every page of a plain list, each item as Graph gives it.
    async fn list_values(&self, first: String) -> Result<Vec<serde_json::Value>, GraphError> {
        let mut url = first;
        let mut all = Vec::new();
        loop {
            let page: serde_json::Value = decode(&self.get(&url, Some(100)).await?)?;
            all.extend(page["value"].as_array().cloned().unwrap_or_default());
            match page["@odata.nextLink"].as_str() {
                // A list without end is a server gone wrong, and is not followed for ever.
                Some(next) if all.len() < 50_000 => url = next.to_owned(),
                _ => return Ok(all),
            }
        }
    }

    /// The folders of contacts beside the main one, as `(id, name)` ("List
    /// contactFolders", SOURCES.md).
    pub async fn contact_folders(&self) -> Result<Vec<(String, String)>, GraphError> {
        let all = self
            .list_values(format!("{}/me/contactFolders", self.base))
            .await?;

        Ok(all
            .iter()
            .filter_map(|each| {
                Some((
                    each["id"].as_str()?.to_owned(),
                    each["displayName"].as_str().unwrap_or_default().to_owned(),
                ))
            })
            .collect())
    }

    /// Every contact in a folder, or with none named in the main Contacts
    /// folder ("List contacts", SOURCES.md).
    pub async fn contacts(
        &self,
        folder_id: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, GraphError> {
        self.list_values(match folder_id {
            Some(folder) => format!("{}/me/contactFolders/{folder}/contacts", self.base),
            None => format!("{}/me/contacts", self.base),
        })
        .await
    }

    /// Make a contact ("Create contact", SOURCES.md). Not safe to repeat
    /// blindly: a second request makes a second contact.
    pub async fn create_contact(
        &self,
        folder_id: Option<&str>,
        contact: &serde_json::Value,
    ) -> Result<serde_json::Value, GraphError> {
        let url = match folder_id {
            Some(folder) => format!("{}/me/contactFolders/{folder}/contacts", self.base),
            None => format!("{}/me/contacts", self.base),
        };

        decode(
            &self
                .send(reqwest::Method::POST, &url, None, Some(contact), false)
                .await?,
        )
    }

    /// Change a contact: only what is given ("Update contact", SOURCES.md).
    pub async fn update_contact(
        &self,
        contact_id: &str,
        changes: &serde_json::Value,
    ) -> Result<serde_json::Value, GraphError> {
        let url = format!("{}/me/contacts/{contact_id}", self.base);

        decode(
            &self
                .send(reqwest::Method::PATCH, &url, None, Some(changes), true)
                .await?,
        )
    }

    /// Delete a contact ("Delete contact", SOURCES.md). One already gone
    /// is as good as deleted.
    pub async fn delete_contact(&self, contact_id: &str) -> Result<(), GraphError> {
        let url = format!("{}/me/contacts/{contact_id}", self.base);

        match self
            .send(reqwest::Method::DELETE, &url, None, None, true)
            .await
        {
            Ok(_) | Err(GraphError::Refused { status: 404, .. }) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// The people most to do with the owner whose names match what was
    /// typed ("List people", SOURCES.md), each as Graph gives it.
    pub async fn people_search(&self, typed: &str) -> Result<Vec<serde_json::Value>, GraphError> {
        let quoted = format!("\"{}\"", typed.replace(['"', '\\'], " "));
        let url = Url::parse_with_params(
            &format!("{}/me/people", self.base),
            [("$search", quoted.as_str()), ("$top", "15")],
        )
        .map_err(|e| GraphError::Decode(e.to_string()))?;
        let page: serde_json::Value = decode(&self.get(url.as_str(), None).await?)?;

        Ok(page["value"].as_array().cloned().unwrap_or_default())
    }

    /// The people of the organisation whose name or address begins with
    /// what was typed ("List users", SOURCES.md), each as Graph gives it.
    pub async fn users_search(&self, typed: &str) -> Result<Vec<serde_json::Value>, GraphError> {
        // In a filter a quote is written twice.
        let typed = typed.replace('\'', "''");
        let filter = format!(
            "startswith(displayName,'{typed}') or startswith(surname,'{typed}') or startswith(mail,'{typed}')"
        );
        let url = Url::parse_with_params(
            &format!("{}/users", self.base),
            [
                ("$filter", filter.as_str()),
                ("$top", "15"),
                (
                    "$select",
                    "id,displayName,givenName,surname,mail,userPrincipalName",
                ),
            ],
        )
        .map_err(|e| GraphError::Decode(e.to_string()))?;
        let page: serde_json::Value = decode(&self.get(url.as_str(), None).await?)?;

        Ok(page["value"].as_array().cloned().unwrap_or_default())
    }

    /// Every meeting room the organisation has ("List place objects",
    /// SOURCES.md), each as Graph gives it.
    pub async fn rooms(&self) -> Result<Vec<serde_json::Value>, GraphError> {
        let mut url = format!("{}/places/microsoft.graph.room", self.base);
        let mut all = Vec::new();
        loop {
            let page: serde_json::Value = decode(&self.get(&url, Some(100)).await?)?;
            all.extend(page["value"].as_array().cloned().unwrap_or_default());
            match page["@odata.nextLink"].as_str() {
                // No organisation has rooms without end; a server that says so is not believed.
                Some(next) if all.len() < 5_000 => url = next.to_owned(),
                _ => return Ok(all),
            }
        }
    }

    /// Times the server suggests for a meeting ("user: findMeetingTimes",
    /// SOURCES.md), as it gives them. It changes nothing, so it is safe to repeat.
    pub async fn find_meeting_times(
        &self,
        asked: &serde_json::Value,
    ) -> Result<serde_json::Value, GraphError> {
        let url = format!("{}/me/findMeetingTimes", self.base);

        decode(
            &self
                .send(reqwest::Method::POST, &url, None, Some(asked), true)
                .await?,
        )
    }

    /// When people are free and busy between two moments in UTC, in slots
    /// of so many minutes ("calendar: getSchedule", SOURCES.md): one entry
    /// for each address asked about, as Graph gives it. It changes nothing,
    /// so it is safe to repeat.
    pub async fn schedules(
        &self,
        addresses: &[String],
        start: &str,
        end: &str,
        interval_minutes: u32,
    ) -> Result<Vec<serde_json::Value>, GraphError> {
        let url = format!("{}/me/calendar/getSchedule", self.base);
        let at = |moment: &str| serde_json::json!({ "dateTime": moment.trim_end_matches('Z'), "timeZone": "UTC" });
        let body = serde_json::json!({
            "schedules": addresses,
            "startTime": at(start),
            "endTime": at(end),
            "availabilityViewInterval": interval_minutes,
        });
        let answer: serde_json::Value = decode(
            &self
                .send(reqwest::Method::POST, &url, None, Some(&body), true)
                .await?,
        )?;

        Ok(answer["value"].as_array().cloned().unwrap_or_default())
    }

    /// An event whole, as Graph gives it: for a series, its master, with
    /// how it repeats ("Get event", SOURCES.md).
    pub async fn event_raw(&self, event_id: &str) -> Result<serde_json::Value, GraphError> {
        let url = format!("{}/me/events/{event_id}", self.base);

        decode(&self.get(&url, None).await?)
    }

    /// Make an event, in a calendar or with none named in the default one
    /// ("Create event", SOURCES.md). Gives back the event as made.
    ///
    /// It is given a `transactionId` of its own, which the server uses to
    /// make one event however many times the request reaches it, so it is
    /// safe to repeat.
    pub async fn create_event(
        &self,
        calendar_id: Option<&str>,
        event: &serde_json::Value,
    ) -> Result<Event, GraphError> {
        let url = match calendar_id {
            Some(calendar) => format!("{}/me/calendars/{calendar}/events", self.base),
            None => format!("{}/me/events", self.base),
        };
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|e| GraphError::Decode(e.to_string()))?;
        let mut event = event.clone();
        event["transactionId"] =
            serde_json::Value::String(bytes.iter().map(|byte| format!("{byte:02x}")).collect());

        decode(
            &self
                .send(reqwest::Method::POST, &url, None, Some(&event), true)
                .await?,
        )
    }

    /// Change an event: only what is given is changed ("Update event",
    /// SOURCES.md). Gives back the event as it now is.
    pub async fn update_event(
        &self,
        event_id: &str,
        changes: &serde_json::Value,
    ) -> Result<Event, GraphError> {
        let url = format!("{}/me/events/{event_id}", self.base);

        decode(
            &self
                .send(reqwest::Method::PATCH, &url, None, Some(changes), true)
                .await?,
        )
    }

    /// Delete an event ("Delete event", SOURCES.md). If it is a meeting
    /// the owner organised, the server tells those invited it is off. One
    /// that is already gone is as good as deleted.
    pub async fn delete_event(&self, event_id: &str) -> Result<(), GraphError> {
        let url = format!("{}/me/events/{event_id}", self.base);

        match self
            .send(reqwest::Method::DELETE, &url, None, None, true)
            .await
        {
            Ok(_) | Err(GraphError::Refused { status: 404, .. }) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// Cancel a meeting the owner organised, with words to those invited
    /// ("event: cancel", SOURCES.md). The event leaves the calendar.
    ///
    /// Not safe to repeat blindly: the cancellation is mail to everyone.
    pub async fn cancel_event(&self, event_id: &str, comment: &str) -> Result<(), GraphError> {
        let url = format!("{}/me/events/{event_id}/cancel", self.base);
        let body = serde_json::json!({ "comment": comment });
        self.send(reqwest::Method::POST, &url, None, Some(&body), false)
            .await?;

        Ok(())
    }

    /// Answer an invitation ("event: accept", "event: tentativelyAccept"
    /// and "event: decline", SOURCES.md). `answer` is the last part of the
    /// address: `accept`, `tentativelyAccept` or `decline`. With
    /// `send_response` the organiser is sent the answer, with the comment.
    ///
    /// Not safe to repeat blindly: a second request sends a second answer.
    pub async fn respond_to_event(
        &self,
        event_id: &str,
        answer: &str,
        comment: &str,
        send_response: bool,
        proposed: Option<(&str, &str)>,
    ) -> Result<(), GraphError> {
        let url = format!("{}/me/events/{event_id}/{answer}", self.base);
        let mut body = serde_json::json!({ "comment": comment, "sendResponse": send_response });
        // Another time, proposed with the answer: its start and end in UTC
        // ("Propose new meeting times", SOURCES.md).
        if let Some((start, end)) = proposed {
            let at = |moment: &str| serde_json::json!({ "dateTime": moment.trim_end_matches('Z'), "timeZone": "UTC" });
            body["proposedNewTime"] = serde_json::json!({ "start": at(start), "end": at(end) });
        }
        self.send(reqwest::Method::POST, &url, None, Some(&body), false)
            .await?;

        Ok(())
    }

    /// The id of one of the folders every mailbox has, by its well-known
    /// name. Nothing if this mailbox has no such folder.
    pub async fn well_known_folder(&self, name: &str) -> Result<Option<String>, GraphError> {
        let url = format!("{}/me/mailFolders/{name}?$select=id", self.base);

        match self.get(&url, None).await {
            Ok(body) => Ok(Some(decode::<MailFolder>(&body)?.id)),
            Err(GraphError::Refused { status: 404, .. }) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// A message's body, as it was sent.
    pub async fn message_body(&self, message_id: &str) -> Result<ItemBody, GraphError> {
        let url = format!("{}/me/messages/{message_id}?$select=id,body", self.base);
        let message: Message = decode(&self.get(&url, None).await?)?;

        message
            .body
            .ok_or_else(|| GraphError::Decode("a message with no body".to_owned()))
    }

    /// What is attached to a message, without the contents. The contents
    /// are left out by naming the other properties: a list would otherwise
    /// carry every file whole.
    pub async fn attachments(&self, message_id: &str) -> Result<Vec<Attachment>, GraphError> {
        let mut url = format!(
            "{}/me/messages/{message_id}/attachments?$select={ATTACHMENT_SELECT}",
            self.base
        );
        let mut all = Vec::new();
        loop {
            let page: AttachmentPage = decode(&self.get(&url, None).await?)?;
            all.extend(page.value);
            match page.next_link {
                Some(next) => url = next,
                None => return Ok(all),
            }
        }
    }

    /// One file attachment whole: its contents, and the name a body refers
    /// to it by. For the pictures within a message, which are small.
    pub async fn file_attachment(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<FileAttachment, GraphError> {
        let url = format!(
            "{}/me/messages/{message_id}/attachments/{attachment_id}",
            self.base
        );

        decode(&self.get(&url, None).await?)
    }

    /// The contents of a file attachment as they were attached, or of an
    /// attached message, event or contact in its MIME form. A reference
    /// attachment has none, and Graph refuses with 405.
    pub async fn attachment_content(
        &self,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<Vec<u8>, GraphError> {
        let url = format!(
            "{}/me/messages/{message_id}/attachments/{attachment_id}/$value",
            self.base
        );

        self.get(&url, None).await
    }

    /// Change properties of a message: `isRead`, `flag` and the others
    /// "Update message" (SOURCES.md) lists. Setting a property to a value
    /// twice leaves it as setting it once does, so this is safe to send
    /// again after an answer that never arrived.
    pub async fn update_message(
        &self,
        message_id: &str,
        changes: &serde_json::Value,
    ) -> Result<(), GraphError> {
        let url = format!("{}/me/messages/{message_id}", self.base);
        self.send(reqwest::Method::PATCH, &url, None, Some(changes), true)
            .await?;

        Ok(())
    }

    /// Move a message to another folder, named by its id or by a well-known
    /// name such as `deleteditems` ("message: move", SOURCES.md).
    ///
    /// A move is not safe to repeat blindly: if the answer is lost, the move
    /// may or may not have happened. So a request that gets no answer is not
    /// sent again here; the caller finds out with `message_folder` first.
    pub async fn move_message(
        &self,
        message_id: &str,
        destination: &str,
    ) -> Result<(), GraphError> {
        let url = format!("{}/me/messages/{message_id}/move", self.base);
        let body = serde_json::json!({ "destinationId": destination });
        self.send(reqwest::Method::POST, &url, None, Some(&body), false)
            .await?;

        Ok(())
    }

    /// Create a draft from a message object: subject, body and recipients
    /// ("Create message", SOURCES.md). Gives back the draft as created.
    ///
    /// Not safe to repeat blindly: a second request makes a second draft.
    pub async fn create_draft(&self, message: &serde_json::Value) -> Result<Message, GraphError> {
        let url = format!("{}/me/messages", self.base);

        decode(
            &self
                .send(reqwest::Method::POST, &url, None, Some(message), false)
                .await?,
        )
    }

    /// Create a draft that answers or forwards a message: `createReply`,
    /// `createReplyAll` or `createForward`. The draft comes back with the
    /// original quoted in its body and, for a reply, its recipients filled in.
    pub async fn create_draft_from(
        &self,
        message_id: &str,
        action: &str,
    ) -> Result<Message, GraphError> {
        let url = format!("{}/me/messages/{message_id}/{action}", self.base);

        decode(
            &self
                .send(reqwest::Method::POST, &url, None, None, false)
                .await?,
        )
    }

    /// Attach a file of under 3 MB to a draft ("Add attachment",
    /// SOURCES.md). Not safe to repeat blindly: a second request attaches
    /// the file a second time.
    pub async fn add_attachment(
        &self,
        message_id: &str,
        name: &str,
        content_type: &str,
        content: &[u8],
        content_id: Option<&str>,
    ) -> Result<(), GraphError> {
        use base64::Engine;

        let url = format!("{}/me/messages/{message_id}/attachments", self.base);
        let mut body = serde_json::json!({
            "@odata.type": "#microsoft.graph.fileAttachment",
            "name": name,
            "contentType": content_type,
            "contentBytes": base64::engine::general_purpose::STANDARD.encode(content),
        });
        // A picture shown within the body: the body refers to it by this
        // name, as `cid:` and the name ("fileAttachment resource type").
        if let Some(content_id) = content_id {
            body["isInline"] = serde_json::json!(true);
            body["contentId"] = serde_json::json!(content_id);
        }
        self.send(reqwest::Method::POST, &url, None, Some(&body), false)
            .await?;

        Ok(())
    }

    /// Attach a file of 3 MB or more to a draft, a piece at a time
    /// ("attachment: createUploadSession" and "Attach large files to
    /// Outlook messages", SOURCES.md).
    ///
    /// The address the pieces go to carries its own authorisation, so it is
    /// sent no token of ours and is never put in an error or a log. If a
    /// piece fails the whole upload fails: the caller starts again, having
    /// first looked whether the file arrived after all.
    pub async fn upload_attachment(
        &self,
        message_id: &str,
        name: &str,
        content: &[u8],
        content_id: Option<&str>,
    ) -> Result<(), GraphError> {
        // "Keep each byte range less than 4 MB."
        const PIECE: usize = 3 * 1024 * 1024;

        let url = format!(
            "{}/me/messages/{message_id}/attachments/createUploadSession",
            self.base
        );
        let mut item = serde_json::json!({
            "AttachmentItem": { "attachmentType": "file", "name": name, "size": content.len() }
        });
        if let Some(content_id) = content_id {
            item["AttachmentItem"]["isInline"] = serde_json::json!(true);
            item["AttachmentItem"]["contentId"] = serde_json::json!(content_id);
        }
        let session: UploadSession = decode(
            &self
                .send(reqwest::Method::POST, &url, None, Some(&item), false)
                .await?,
        )?;
        let upload_url = Url::parse(&session.upload_url)
            .map_err(|_| GraphError::Decode("an upload address that is not a URL".to_owned()))?;

        let total = content.len();
        let mut start = 0;
        while start < total {
            let end = (start + PIECE).min(total);
            let _permit = self
                .in_flight
                .acquire()
                .await
                .map_err(|e| GraphError::Decode(e.to_string()))?;
            let response = self
                .http
                .put(upload_url.clone())
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .header(
                    reqwest::header::CONTENT_RANGE,
                    format!("bytes {start}-{}/{total}", end - 1),
                )
                .body(content[start..end].to_vec())
                .send()
                .await
                .map_err(|e| GraphError::Network(e.without_url()))?;

            let status = response.status();
            if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                return Err(GraphError::Busy {
                    status: status.as_u16(),
                });
            }
            if !status.is_success() {
                return Err(GraphError::Refused {
                    status: status.as_u16(),
                    code: String::new(),
                    message: "The file could not be uploaded.".to_owned(),
                });
            }
            start = end;
        }

        Ok(())
    }

    /// The mailbox's automatic replies as Graph gives them
    /// ("automaticRepliesSetting resource type", SOURCES.md).
    pub async fn automatic_replies(&self) -> Result<serde_json::Value, GraphError> {
        let url = format!("{}/me/mailboxSettings/automaticRepliesSetting", self.base);

        decode(&self.get(&url, None).await?)
    }

    /// Set the mailbox's automatic replies ("Update user mailbox
    /// settings"). Setting them twice leaves them as setting them once does.
    pub async fn set_automatic_replies(
        &self,
        setting: &serde_json::Value,
    ) -> Result<(), GraphError> {
        let url = format!("{}/me/mailboxSettings", self.base);
        let body = serde_json::json!({ "automaticRepliesSetting": setting });
        self.send(reqwest::Method::PATCH, &url, None, Some(&body), true)
            .await?;

        Ok(())
    }

    /// The categories the mailbox has, each a name and a colour
    /// ("List masterCategories", SOURCES.md), as `(name, colour)` where the
    /// colour is `none` or `preset0` to `preset24`.
    pub async fn master_categories(&self) -> Result<Vec<(String, String)>, GraphError> {
        let url = format!("{}/me/outlook/masterCategories", self.base);
        let list: serde_json::Value = decode(&self.get(&url, None).await?)?;

        Ok(list["value"]
            .as_array()
            .map(|all| {
                all.iter()
                    .filter_map(|each| {
                        Some((
                            each["displayName"].as_str()?.to_owned(),
                            each["color"]
                                .as_str()
                                .unwrap_or("none")
                                .to_ascii_lowercase(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Send a draft ("message: send", SOURCES.md). It is saved in Sent
    /// Items. Never repeated here: a second request could send a second copy.
    pub async fn send_draft(&self, message_id: &str) -> Result<(), GraphError> {
        let url = format!("{}/me/messages/{message_id}/send", self.base);
        self.send(reqwest::Method::POST, &url, None, None, false)
            .await?;

        Ok(())
    }

    /// A draft whole, as it is on the server: its subject, its body and
    /// everyone it is addressed to, the blind copies included.
    pub async fn draft(&self, message_id: &str) -> Result<Message, GraphError> {
        let url = format!(
            "{}/me/messages/{message_id}?$select=id,subject,body,toRecipients,ccRecipients,bccRecipients,isDraft",
            self.base
        );

        decode(&self.get(&url, None).await?)
    }

    /// Whether a message is still a draft. Nothing if it is no longer there.
    pub async fn is_draft(&self, message_id: &str) -> Result<Option<bool>, GraphError> {
        let url = format!("{}/me/messages/{message_id}?$select=id,isDraft", self.base);

        match self.get(&url, None).await {
            Ok(body) => Ok(Some(decode::<Message>(&body)?.is_draft.unwrap_or(false))),
            Err(GraphError::Refused { status: 404, .. }) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// Delete a message for good ("message: permanentDelete", SOURCES.md):
    /// it goes where no mail program can reach it, and is purged in time.
    /// A message already gone counts as deleted, so asking twice is safe.
    pub async fn delete_permanently(&self, message_id: &str) -> Result<(), GraphError> {
        let url = format!("{}/me/messages/{message_id}/permanentDelete", self.base);

        match self
            .send(reqwest::Method::POST, &url, None, None, true)
            .await
        {
            Ok(_) | Err(GraphError::Refused { status: 404, .. }) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// A message whole, in MIME: what a `.eml` file holds ("Get MIME
    /// content of a message", SOURCES.md).
    pub async fn message_mime(&self, message_id: &str) -> Result<Vec<u8>, GraphError> {
        let url = format!("{}/me/messages/{message_id}/$value", self.base);

        self.get(&url, None).await
    }

    /// A message about a meeting, whole, with the event in the calendar
    /// that it is about ("Get eventMessage", SOURCES.md). As Graph gives it:
    /// what it holds differs between an invitation and an answer to one.
    pub async fn meeting_message(&self, message_id: &str) -> Result<serde_json::Value, GraphError> {
        let url = format!(
            "{}/me/messages/{message_id}?$expand=microsoft.graph.eventMessage/event",
            self.base
        );

        decode(&self.get(&url, None).await?)
    }

    /// The id of the folder a message is in now.
    pub async fn message_folder(&self, message_id: &str) -> Result<Option<String>, GraphError> {
        let url = format!(
            "{}/me/messages/{message_id}?$select=id,parentFolderId",
            self.base
        );
        let message: Message = decode(&self.get(&url, None).await?)?;

        Ok(message.parent_folder_id)
    }

    /// GET a URL. A GET changes nothing, so it is always safe to send again.
    async fn get(&self, url: &str, page_size: Option<u32>) -> Result<Vec<u8>, GraphError> {
        self.send(reqwest::Method::GET, url, page_size, None, true)
            .await
    }

    /// Send a request, waiting and trying again while Graph says to wait.
    /// `repeatable` says whether it may also be sent again when no answer
    /// came at all, which is only so for a request that leaves things the
    /// same however many times it arrives.
    async fn send(
        &self,
        method: reqwest::Method,
        url: &str,
        page_size: Option<u32>,
        body: Option<&serde_json::Value>,
        repeatable: bool,
    ) -> Result<Vec<u8>, GraphError> {
        let url = Url::parse(&self.addressed(url))
            .map_err(|e| GraphError::Decode(format!("not a URL: {e}")))?;
        // Closing the semaphore is not something this client ever does.
        let _permit = self
            .in_flight
            .acquire()
            .await
            .map_err(|e| GraphError::Decode(e.to_string()))?;

        let mut attempt = 0;
        loop {
            let token = self.tokens.token().await?;
            // "Get immutable identifiers for Outlook resources" (SOURCES.md):
            // with this header an id survives a move between folders.
            let mut prefer = "IdType=\"ImmutableId\"".to_owned();
            if let Some(size) = page_size {
                prefer.push_str(&format!(", odata.maxpagesize={size}"));
            }

            let mut request = self
                .http
                .request(method.clone(), url.clone())
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .header("Prefer", prefer);
            request = match body {
                Some(body) => request.json(body),
                // "message: send" asks for a Content-Length of 0 by name, and
                // Graph answers 411 without one. It is set outright: left
                // to the HTTP library, a request with nothing in it goes
                // out with no length at all.
                None if method == reqwest::Method::POST => request
                    .header(reqwest::header::CONTENT_LENGTH, "0")
                    .body(Vec::new()),
                None => request,
            };
            let sent = request.send().await;

            let last_try = attempt + 1 >= self.retry.attempts;
            let response = match sent {
                Ok(response) => response,
                Err(error) if last_try || !repeatable => return Err(GraphError::Network(error)),
                Err(_) => {
                    tokio::time::sleep(self.retry.wait(attempt, None)).await;
                    attempt += 1;
                    continue;
                }
            };

            let status = response.status();
            if status.is_success() {
                return response
                    .bytes()
                    .await
                    .map(|b| b.to_vec())
                    .map_err(GraphError::Network);
            }

            // Throttled: the request was turned away before anything was
            // done, so any request may be sent again. Unavailable or timed
            // out at a gateway: probably not done, but not certainly, so
            // only a repeatable request is sent again.
            let wait = status == StatusCode::TOO_MANY_REQUESTS
                || (repeatable
                    && matches!(
                        status,
                        StatusCode::SERVICE_UNAVAILABLE | StatusCode::GATEWAY_TIMEOUT
                    ));
            if wait && !last_try {
                let retry_after = response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse().ok());
                tokio::time::sleep(self.retry.wait(attempt, retry_after)).await;
                attempt += 1;
                continue;
            }
            // Any failure on the server's side is one that may pass, and
            // says nothing certain about whether the request was carried
            // out. Only a 4xx is the server's considered no.
            if wait || status.is_server_error() {
                return Err(GraphError::Busy {
                    status: status.as_u16(),
                });
            }

            return Err(refusal(response).await);
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadSession {
    upload_url: String,
}

#[derive(Deserialize)]
struct AttachmentPage {
    #[serde(default)]
    value: Vec<Attachment>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

/// Turn a response that is neither a success nor a "wait" into its error.
async fn refusal(response: reqwest::Response) -> GraphError {
    let status = response.status();
    let challenge = response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = response.bytes().await.unwrap_or_default();
    let detail = serde_json::from_slice::<ErrorBody>(&body)
        .map(|b| b.error)
        .unwrap_or(ErrorDetail {
            code: String::new(),
            message: String::new(),
        });

    // "Use delta query to track changes" (SOURCES.md): a 410, or an error
    // such as syncStateNotFound, means the saved place is no longer good.
    if status == StatusCode::GONE || detail.code.eq_ignore_ascii_case("syncStateNotFound") {
        return GraphError::SyncStateGone;
    }
    if status == StatusCode::UNAUTHORIZED {
        return GraphError::Unauthorized { claims: challenge };
    }

    GraphError::Refused {
        status: status.as_u16(),
        code: detail.code,
        message: detail.message,
    }
}

fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, GraphError> {
    serde_json::from_slice(body).map_err(|e| GraphError::Decode(e.to_string()))
}
