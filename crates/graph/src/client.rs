use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use reqwest::header::{AUTHORIZATION, RETRY_AFTER, WWW_AUTHENTICATE};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::Semaphore;

use crate::delta::read_page;
use crate::mail::{ATTACHMENT_SELECT, FOLDER_SELECT, MESSAGE_SELECT};
use crate::{
    Attachment, DeltaPage, FileAttachment, GraphError, ItemBody, MailFolder, Message, RetryPolicy,
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
        })
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
        self.send(reqwest::Method::PATCH, &url, None, Some(changes))
            .await?;

        Ok(())
    }

    /// GET a URL. A GET changes nothing, so it is always safe to send again.
    async fn get(&self, url: &str, page_size: Option<u32>) -> Result<Vec<u8>, GraphError> {
        self.send(reqwest::Method::GET, url, page_size, None).await
    }

    /// Send a request, waiting and trying again while Graph says to. Only
    /// for requests that are safe to repeat.
    async fn send(
        &self,
        method: reqwest::Method,
        url: &str,
        page_size: Option<u32>,
        body: Option<&serde_json::Value>,
    ) -> Result<Vec<u8>, GraphError> {
        let url = Url::parse(url).map_err(|e| GraphError::Decode(format!("not a URL: {e}")))?;
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
            if let Some(body) = body {
                request = request.json(body);
            }
            let sent = request.send().await;

            let last_try = attempt + 1 >= self.retry.attempts;
            let response = match sent {
                Ok(response) => response,
                Err(error) if last_try => return Err(GraphError::Network(error)),
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

            if matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS
                    | StatusCode::SERVICE_UNAVAILABLE
                    | StatusCode::GATEWAY_TIMEOUT
            ) {
                if last_try {
                    return Err(GraphError::Busy {
                        status: status.as_u16(),
                    });
                }
                let retry_after = response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse().ok());
                tokio::time::sleep(self.retry.wait(attempt, retry_after)).await;
                attempt += 1;
                continue;
            }

            return Err(refusal(response).await);
        }
    }
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
