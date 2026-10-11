//! A stand-in for Microsoft Graph.
//!
//! It listens on a port of its own and answers the way Graph is documented
//! to, so the client's sync and outbox can be tested without a network or a
//! tenant, and can be made to meet the failures a real one only produces now
//! and then: throttling, and a service that is briefly down.
//!
//! It knows the signed-in user and their mail folders and messages, delta
//! queries included. Each resource the client comes to use is added here
//! first.

mod identity;
mod mailbox;

pub use identity::FakeIdentity;
pub use mailbox::{FakeAttachment, FakeMessage};

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::Router;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};

/// A failure to hand back in place of the real answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// 429 with a `Retry-After` of this many seconds.
    Throttled { retry_after_secs: u32 },
    /// 503, with no hint of when to come back.
    Unavailable,
}

#[derive(Default)]
struct Inner {
    /// Handed out one a request, oldest first.
    faults: Vec<Fault>,
    /// How many requests to answer normally before handing out faults.
    pass_first: usize,
    /// Every request received, as `GET /v1.0/me`.
    requests: Vec<String>,
    /// Out of every thousand requests, how many to fail at random, and the
    /// state of the generator that decides which.
    chaos: Option<(u32, u64)>,
    mailbox: mailbox::Mailbox,
    /// How many moves have been made.
    moves: u32,
    /// Every message sent, as it was when it went, oldest first.
    sent: Vec<FakeMessage>,
    drafts_created: u32,
    /// Send the next draft and then answer as if it had failed.
    lose_next_send_answer: bool,
    /// Make the next move and then answer as if it had failed.
    lose_next_move_answer: bool,
    /// Messages every change to which is refused.
    refused_updates: std::collections::HashSet<String>,
    /// Where this server is, to write links to itself.
    base: String,
}

type Shared = Arc<Mutex<Inner>>;

pub struct FakeGraph {
    address: SocketAddr,
    shared: Shared,
    server: tokio::task::JoinHandle<()>,
}

impl FakeGraph {
    /// Start listening on a free local port.
    pub async fn start() -> std::io::Result<Self> {
        let shared = Shared::default();
        let app = Router::new()
            .route("/v1.0/me", get(me))
            .route("/v1.0/me/mailFolders/delta", get(folders_delta))
            .route("/v1.0/me/mailFolders/{folder}", get(folder))
            .route(
                "/v1.0/me/mailFolders/{folder}/messages/delta",
                get(messages_delta),
            )
            .route(
                "/v1.0/me/messages/{message}",
                get(message).patch(update_message),
            )
            .route(
                "/v1.0/me/messages/{message}/move",
                axum::routing::post(move_message),
            )
            .route("/v1.0/me/messages", axum::routing::post(create_draft))
            .route(
                "/v1.0/me/messages/{message}/send",
                axum::routing::post(send_draft),
            )
            .route(
                "/v1.0/me/messages/{message}/createReply",
                axum::routing::post(create_reply),
            )
            .route(
                "/v1.0/me/messages/{message}/createReplyAll",
                axum::routing::post(create_reply_all),
            )
            .route(
                "/v1.0/me/messages/{message}/createForward",
                axum::routing::post(create_forward),
            )
            .route("/v1.0/me/messages/{message}/attachments", get(attachments))
            .route(
                "/v1.0/me/messages/{message}/attachments/{attachment}",
                get(attachment),
            )
            .route(
                "/v1.0/me/messages/{message}/attachments/{attachment}/$value",
                get(attachment_content),
            )
            .fallback(not_built)
            // Outermost last: a request is logged, then faulted, then checked for a token.
            .layer(middleware::from_fn(require_token))
            .layer(middleware::from_fn_with_state(shared.clone(), inject_fault))
            .layer(middleware::from_fn_with_state(shared.clone(), record))
            .with_state(shared.clone());

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        lock(&shared).base = format!("http://{address}/v1.0");
        let server = tokio::spawn(async move {
            // The server only stops when the test drops it.
            let _ = axum::serve(listener, app).await;
        });

        Ok(Self {
            address,
            shared,
            server,
        })
    }

    /// What to use in place of `https://graph.microsoft.com/v1.0`.
    pub fn base_url(&self) -> String {
        format!("http://{}/v1.0", self.address)
    }

    /// Answer the next requests with these faults, one each, before answering normally again.
    pub fn fail_next(&self, faults: impl IntoIterator<Item = Fault>) {
        lock(&self.shared).faults.extend(faults);
    }

    /// Answer the next `pass` requests normally, and then the ones after
    /// with these faults: an outage that begins part-way through something.
    pub fn fail_after(&self, pass: usize, faults: impl IntoIterator<Item = Fault>) {
        let mut inner = lock(&self.shared);
        inner.pass_first = pass;
        inner.faults.extend(faults);
    }

    /// Forget any faults not yet handed out.
    pub fn clear_faults(&self) {
        let mut inner = lock(&self.shared);
        inner.pass_first = 0;
        inner.faults.clear();
    }

    /// From now on fail this many requests in every thousand, at random but
    /// the same way each time for the same seed, with a 429 or a 503.
    pub fn fail_at_random(&self, per_thousand: u32, seed: u64) {
        lock(&self.shared).chaos = Some((per_thousand, seed.max(1)));
    }

    pub fn stop_failing(&self) {
        let mut inner = lock(&self.shared);
        inner.chaos = None;
        inner.faults.clear();
    }

    /// The id of a folder every mailbox has, by its well-known name.
    pub fn folder_id(&self, well_known: &str) -> String {
        lock(&self.shared)
            .mailbox
            .folder_id(well_known)
            .unwrap_or_default()
    }

    pub fn add_folder(&self, name: &str, parent: Option<&str>) -> String {
        lock(&self.shared).mailbox.add_folder(name, parent)
    }

    pub fn rename_folder(&self, id: &str, name: &str) {
        lock(&self.shared).mailbox.rename_folder(id, name);
    }

    pub fn remove_folder(&self, id: &str) {
        lock(&self.shared).mailbox.remove_folder(id);
    }

    /// The next move is made, but the answer to it is an error: the case
    /// of a request that reached the server and an answer that did not come back.
    pub fn lose_next_move_answer(&self) {
        lock(&self.shared).lose_next_move_answer = true;
    }

    /// Every message the server has sent, as it was when it went.
    pub fn sent(&self) -> Vec<FakeMessage> {
        lock(&self.shared).sent.clone()
    }

    /// How many drafts have been created on the server.
    pub fn drafts_created(&self) -> u32 {
        lock(&self.shared).drafts_created
    }

    /// The next draft is sent, but the answer to the request is an error.
    pub fn lose_next_send_answer(&self) {
        lock(&self.shared).lose_next_send_answer = true;
    }

    /// How many moves the server has made.
    pub fn moves(&self) -> u32 {
        lock(&self.shared).moves
    }

    /// From now on refuse, for good, every change to this message.
    pub fn refuse_updates_to(&self, message: &str) {
        lock(&self.shared)
            .refused_updates
            .insert(message.to_owned());
    }

    /// A message's flag, as the server has it.
    pub fn flag(&self, message: &str) -> Option<String> {
        lock(&self.shared).mailbox.flag(message)
    }

    pub fn add_message(&self, folder: &str, message: FakeMessage) -> String {
        lock(&self.shared).mailbox.add_message(folder, message)
    }

    /// Fill a folder with this many made-up messages, newest last added.
    pub fn add_messages(&self, folder: &str, count: usize) -> Vec<String> {
        let mut inner = lock(&self.shared);
        (0..count)
            .map(|n| {
                let day = 1 + (n / 1440) % 28;
                let (hour, minute) = ((n / 60) % 24, n % 60);
                let message = FakeMessage::new(
                    &format!("Message number {n}"),
                    &format!("Sender {}", n % 97),
                    &format!("sender{}@example.test", n % 97),
                    &format!(
                        "2026-{:02}-{day:02}T{hour:02}:{minute:02}:00Z",
                        1 + (n / 40320) % 12
                    ),
                );
                inner.mailbox.add_message(folder, message)
            })
            .collect()
    }

    pub fn set_read(&self, id: &str, is_read: bool) {
        lock(&self.shared).mailbox.set_read(id, is_read);
    }

    pub fn set_subject(&self, id: &str, subject: &str) {
        lock(&self.shared).mailbox.set_subject(id, subject);
    }

    pub fn move_message(&self, id: &str, to_folder: &str) {
        lock(&self.shared).mailbox.move_message(id, to_folder);
    }

    pub fn delete_message(&self, id: &str) {
        lock(&self.shared).mailbox.delete_message(id);
    }

    /// Make every delta link handed out so far stale, as Graph does when its
    /// token cache turns over: the next use of one is answered 410.
    pub fn expire_delta_tokens(&self) {
        lock(&self.shared).mailbox.expire_delta_tokens();
    }

    /// Answer every delta request as stale, a first one included: a server
    /// that never lets a sync finish.
    pub fn never_settle(&self, never: bool) {
        lock(&self.shared).mailbox.never_settles = never;
    }

    /// Every message as it truly is on the server: id to (folder, subject, read).
    pub fn messages(&self) -> BTreeMap<String, (String, String, bool)> {
        lock(&self.shared).mailbox.messages()
    }

    /// Every folder as it truly is on the server: id to (name, parent).
    pub fn folders(&self) -> BTreeMap<String, (String, String)> {
        lock(&self.shared).mailbox.folders()
    }

    /// Every request received so far, oldest first, as `GET /v1.0/me`.
    pub fn requests(&self) -> Vec<String> {
        lock(&self.shared).requests.clone()
    }
}

impl Drop for FakeGraph {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// A test that panicked while holding the lock has already failed; the data is still good to read.
fn lock(shared: &Shared) -> MutexGuard<'_, Inner> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn record(State(shared): State<Shared>, request: Request, next: Next) -> Response {
    lock(&shared)
        .requests
        .push(format!("{} {}", request.method(), request.uri().path()));
    next.run(request).await
}

async fn inject_fault(State(shared): State<Shared>, request: Request, next: Next) -> Response {
    let fault = {
        let mut inner = lock(&shared);
        if inner.pass_first > 0 {
            inner.pass_first -= 1;
            None
        } else if !inner.faults.is_empty() {
            Some(inner.faults.remove(0))
        } else if let Some((per_thousand, state)) = inner.chaos.as_mut() {
            // xorshift: small, and the same sequence for the same seed.
            *state ^= *state << 13;
            *state ^= *state >> 7;
            *state ^= *state << 17;
            let roll = (*state % 1000) as u32;
            let fault = if roll.is_multiple_of(2) {
                Fault::Throttled {
                    retry_after_secs: 0,
                }
            } else {
                Fault::Unavailable
            };
            (roll < *per_thousand).then_some(fault)
        } else {
            None
        }
    };

    match fault {
        // The status, the header and the body's shape are as documented under
        // "Microsoft Graph throttling guidance" (SOURCES.md).
        Some(Fault::Throttled { retry_after_secs }) => {
            let mut response = error(
                StatusCode::TOO_MANY_REQUESTS,
                "TooManyRequests",
                "Please retry again later.",
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after_secs));
            response
        }
        Some(Fault::Unavailable) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "serviceUnavailable",
            "The service is temporarily unavailable.",
        ),
        None => next.run(request).await,
    }
}

async fn require_token(request: Request, next: Next) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|token| !token.is_empty())
        .map(str::to_owned);
    let Some(token) = token else {
        return error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "A bearer token is required.",
        );
    };

    // A token carries the permissions it was issued with after a dot (see
    // the fake identity platform). Mail is read with Mail.Read or
    // Mail.ReadWrite and changed only with Mail.ReadWrite, as the reference
    // page of each request says.
    let granted: Vec<&str> = token
        .split_once('.')
        .map(|(_, scopes)| scopes.split(',').collect())
        .unwrap_or_default();
    let path = request.uri().path();
    let mail = path.contains("/me/mailFolders") || path.contains("/me/messages");
    let reading = request.method() == axum::http::Method::GET;
    // Sending is its own permission ("message: send").
    let allowed = if path.ends_with("/send") {
        granted.contains(&"Mail.Send")
    } else {
        !mail || granted.contains(&"Mail.ReadWrite") || (reading && granted.contains(&"Mail.Read"))
    };
    if !allowed {
        return error(
            StatusCode::FORBIDDEN,
            "ErrorAccessDenied",
            "Access is denied. Check credentials and try again.",
        );
    }

    next.run(request).await
}

/// The signed-in user, with the properties Graph returns by default. The
/// person is made up.
async fn me() -> impl IntoResponse {
    axum::Json(json!({
        "@odata.context": "https://graph.microsoft.com/v1.0/$metadata#users/$entity",
        "businessPhones": ["+61 7 5550 0100"],
        "displayName": "Robin Example",
        "givenName": "Robin",
        "jobTitle": "Office Manager",
        "mail": "robin@example.test",
        "mobilePhone": null,
        "officeLocation": null,
        "preferredLanguage": "en-AU",
        "surname": "Example",
        "userPrincipalName": "robin@example.test",
        "id": "00000000-0000-4000-8000-000000000001"
    }))
}

/// The page size asked for with `Prefer: odata.maxpagesize`, or Graph's own modest default.
fn page_size(request: &axum::http::HeaderMap) -> usize {
    request
        .get_all("prefer")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .find_map(|part| part.trim().strip_prefix("odata.maxpagesize=")?.parse().ok())
        .unwrap_or(10)
}

fn token(query: &HashMap<String, String>) -> Option<&str> {
    query
        .get("$skiptoken")
        .or_else(|| query.get("$deltatoken"))
        .map(String::as_str)
}

fn delta_answer(answer: mailbox::Answer) -> Response {
    match answer {
        mailbox::Answer::Page(body) => axum::Json(body).into_response(),
        // "Synchronization reset" in the delta query overview: 410 Gone.
        mailbox::Answer::Gone => error(
            StatusCode::GONE,
            "syncStateNotFound",
            "The delta token is no longer valid.",
        ),
        mailbox::Answer::NotFound => error(
            StatusCode::NOT_FOUND,
            "itemNotFound",
            "There is no such folder.",
        ),
    }
}

async fn folders_delta(
    State(shared): State<Shared>,
    Query(query): Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let inner = lock(&shared);
    let link = format!("{}/me/mailFolders/delta", inner.base);

    delta_answer(
        inner
            .mailbox
            .folders_delta(token(&query), page_size(&headers), &link),
    )
}

async fn messages_delta(
    State(shared): State<Shared>,
    Path(folder): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let inner = lock(&shared);
    let link = format!("{}/me/mailFolders/{folder}/messages/delta", inner.base);

    delta_answer(
        inner
            .mailbox
            .messages_delta(&folder, token(&query), page_size(&headers), &link),
    )
}

async fn folder(State(shared): State<Shared>, Path(folder): Path<String>) -> Response {
    match lock(&shared).mailbox.folder(&folder) {
        Some(folder) => axum::Json(folder).into_response(),
        None => error(
            StatusCode::NOT_FOUND,
            "itemNotFound",
            "There is no such folder.",
        ),
    }
}

async fn message(State(shared): State<Shared>, Path(message): Path<String>) -> Response {
    match lock(&shared).mailbox.message(&message) {
        Some(message) => axum::Json(message).into_response(),
        None => error(
            StatusCode::NOT_FOUND,
            "itemNotFound",
            "There is no such message.",
        ),
    }
}

/// "Update message" (SOURCES.md): the properties given are changed, the
/// rest left alone, and the message comes back. Only what Brolga changes
/// is built; anything else is refused loudly.
async fn update_message(
    State(shared): State<Shared>,
    Path(message): Path<String>,
    axum::Json(changes): axum::Json<Value>,
) -> Response {
    let mut inner = lock(&shared);
    if inner.mailbox.message(&message).is_none() {
        return error(
            StatusCode::NOT_FOUND,
            "itemNotFound",
            "There is no such message.",
        );
    }
    if inner.refused_updates.contains(&message) {
        return error(
            StatusCode::BAD_REQUEST,
            "fakeGraphRefused",
            "This message cannot be changed.",
        );
    }
    let Some(changes) = changes.as_object() else {
        return error(
            StatusCode::BAD_REQUEST,
            "badRequest",
            "An object is required.",
        );
    };

    let is_draft = inner.mailbox.stored(&message).is_some_and(|m| m.is_draft);
    for (property, value) in changes {
        match (property.as_str(), value) {
            // "Update message": these may be changed only on a draft.
            ("subject" | "body" | "toRecipients" | "ccRecipients" | "bccRecipients", _)
                if !is_draft =>
            {
                return error(
                    StatusCode::BAD_REQUEST,
                    "fakeGraphNotADraft",
                    &format!("`{property}` can be changed only on a draft."),
                );
            }
            ("subject", Value::String(subject)) => {
                let subject = subject.clone();
                inner.mailbox.edit(&message, |m| m.subject = subject);
            }
            ("body", body) if body["content"].is_string() => {
                let content = body["content"].as_str().unwrap_or_default().to_owned();
                inner.mailbox.edit(&message, |m| m.body_html = content);
            }
            ("toRecipients" | "ccRecipients" | "bccRecipients", list) if list.is_array() => {
                let list = people(list);
                inner.mailbox.edit(&message, |m| match property.as_str() {
                    "toRecipients" => m.to = list,
                    "ccRecipients" => m.cc = list,
                    _ => m.bcc = list,
                });
            }
            ("isRead", Value::Bool(read)) => inner.mailbox.set_read(&message, *read),
            ("flag", flag) if flag["flagStatus"].is_string() => {
                let status = flag["flagStatus"].as_str().unwrap_or_default().to_owned();
                inner.mailbox.set_flag(&message, &status);
            }
            _ => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "fakeGraphNotBuilt",
                    &format!("The fake Graph server does not change `{property}` yet."),
                );
            }
        }
    }

    match inner.mailbox.message(&message) {
        Some(updated) => axum::Json(updated).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Recipients as Graph writes them, read into `(name, address)`.
fn people(list: &Value) -> Vec<(String, String)> {
    list.as_array()
        .map(|list| {
            list.iter()
                .map(|each| {
                    let text = |key: &str| {
                        each["emailAddress"][key]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned()
                    };
                    (text("name"), text("address"))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn created(inner: &mut Inner, draft: FakeMessage) -> Response {
    let drafts = inner.mailbox.folder_id("drafts").unwrap_or_default();
    let id = inner.mailbox.add_message(&drafts, draft);
    inner.drafts_created += 1;

    match inner.mailbox.message(&id) {
        Some(message) => (StatusCode::CREATED, axum::Json(message)).into_response(),
        None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn new_draft(subject: &str) -> FakeMessage {
    let mut draft = FakeMessage::new(
        subject,
        "Robin Example",
        "robin@example.test",
        "2026-10-10T00:00:00Z",
    );
    draft.is_draft = true;
    draft.is_read = true;
    draft.body_html = String::new();
    draft
}

/// "Create message" (SOURCES.md): a draft, in Drafts, from a message object.
async fn create_draft(
    State(shared): State<Shared>,
    axum::Json(message): axum::Json<Value>,
) -> Response {
    let mut draft = new_draft(message["subject"].as_str().unwrap_or_default());
    draft.body_html = message["body"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    draft.to = people(&message["toRecipients"]);
    draft.cc = people(&message["ccRecipients"]);
    draft.bcc = people(&message["bccRecipients"]);

    created(&mut lock(&shared), draft)
}

/// A draft answering or forwarding a message: the original quoted in its
/// body, in the same conversation, addressed as the kind of answer needs.
fn create_from(shared: &Shared, message: &str, kind: &str) -> Response {
    let mut inner = lock(shared);
    let Some(original) = inner.mailbox.stored(message) else {
        return error(
            StatusCode::NOT_FOUND,
            "itemNotFound",
            "There is no such message.",
        );
    };

    let prefix = if kind == "forward" { "FW: " } else { "RE: " };
    let mut draft = new_draft(&format!("{prefix}{}", original.subject));
    draft.conversation = Some(
        original
            .conversation
            .clone()
            .unwrap_or_else(|| format!("conversation-{message}")),
    );
    draft.body_html = format!(
        "<hr><p><b>From:</b> {}</p>{}",
        original.from_name, original.body_html
    );
    if kind != "forward" {
        draft.to = vec![(original.from_name.clone(), original.from_address.clone())];
    }
    if kind == "replyAll" {
        draft.cc = original.cc.clone();
    }

    created(&mut inner, draft)
}

async fn create_reply(State(shared): State<Shared>, Path(message): Path<String>) -> Response {
    create_from(&shared, &message, "reply")
}

async fn create_reply_all(State(shared): State<Shared>, Path(message): Path<String>) -> Response {
    create_from(&shared, &message, "replyAll")
}

async fn create_forward(State(shared): State<Shared>, Path(message): Path<String>) -> Response {
    create_from(&shared, &message, "forward")
}

/// "message: send" (SOURCES.md): the draft is sent and saved in Sent
/// Items, and the answer is 202 with nothing in it.
async fn send_draft(State(shared): State<Shared>, Path(message): Path<String>) -> Response {
    let mut inner = lock(&shared);
    let Some(draft) = inner.mailbox.stored(&message) else {
        return error(
            StatusCode::NOT_FOUND,
            "itemNotFound",
            "There is no such message.",
        );
    };
    if !draft.is_draft {
        return error(
            StatusCode::BAD_REQUEST,
            "fakeGraphNotADraft",
            "Only a draft can be sent.",
        );
    }
    if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "ErrorInvalidRecipients",
            "At least one recipient is required.",
        );
    }

    let sent = inner.mailbox.folder_id("sentitems").unwrap_or_default();
    inner.mailbox.edit(&message, |m| m.is_draft = false);
    inner.mailbox.move_message(&message, &sent);
    inner.sent.push(draft);
    if std::mem::take(&mut inner.lose_next_send_answer) {
        // Sent, and the caller never hears of it.
        return StatusCode::BAD_GATEWAY.into_response();
    }

    StatusCode::ACCEPTED.into_response()
}

/// "message: move" (SOURCES.md): to a folder by id or well-known name,
/// answered with 201 and the message. The id does not change, as with
/// immutable ids it does not.
async fn move_message(
    State(shared): State<Shared>,
    Path(message): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut inner = lock(&shared);
    if inner.mailbox.message(&message).is_none() {
        return error(
            StatusCode::NOT_FOUND,
            "itemNotFound",
            "There is no such message.",
        );
    }
    if inner.refused_updates.contains(&message) {
        return error(
            StatusCode::BAD_REQUEST,
            "fakeGraphRefused",
            "This message cannot be changed.",
        );
    }
    let destination = body["destinationId"].as_str().unwrap_or_default();
    let Some(folder) = inner.mailbox.folder_id(destination) else {
        return error(
            StatusCode::NOT_FOUND,
            "itemNotFound",
            "There is no such folder.",
        );
    };
    inner.mailbox.move_message(&message, &folder);
    inner.moves += 1;
    let lose_answer = std::mem::take(&mut inner.lose_next_move_answer);

    match inner.mailbox.message(&message) {
        // The move is made, and the caller never hears of it.
        Some(_) if lose_answer => StatusCode::BAD_GATEWAY.into_response(),
        Some(moved) => (StatusCode::CREATED, axum::Json(moved)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn attachments(
    State(shared): State<Shared>,
    Path(message): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let with_content = query
        .get("$select")
        .is_none_or(|select| select.split(',').any(|each| each == "contentBytes"));

    match lock(&shared).mailbox.attachments(&message, with_content) {
        Some(listed) => axum::Json(listed).into_response(),
        None => error(
            StatusCode::NOT_FOUND,
            "itemNotFound",
            "There is no such message.",
        ),
    }
}

async fn attachment(
    State(shared): State<Shared>,
    Path((message, attachment)): Path<(String, String)>,
) -> Response {
    match lock(&shared).mailbox.attachment_json(&message, &attachment) {
        Some(found) => axum::Json(found).into_response(),
        None => error(
            StatusCode::NOT_FOUND,
            "itemNotFound",
            "There is no such attachment.",
        ),
    }
}

async fn attachment_content(
    State(shared): State<Shared>,
    Path((message, attachment)): Path<(String, String)>,
) -> Response {
    match lock(&shared).mailbox.attachment(&message, &attachment) {
        // "Get attachment" (SOURCES.md): a reference attachment has no value.
        Some(found) if found.kind == "reference" => error(
            StatusCode::METHOD_NOT_ALLOWED,
            "fakeGraphNoValue",
            "A reference attachment has no contents.",
        ),
        Some(found) => (
            [(axum::http::header::CONTENT_TYPE, found.content_type)],
            found.content,
        )
            .into_response(),
        None => error(
            StatusCode::NOT_FOUND,
            "itemNotFound",
            "There is no such attachment.",
        ),
    }
}

/// Said loudly, so a test that wanders off what is built fails on the reason.
async fn not_built(request: Request) -> Response {
    error(
        StatusCode::NOT_FOUND,
        "fakeGraphNotBuilt",
        &format!(
            "The fake Graph server does not answer {} {} yet.",
            request.method(),
            request.uri().path()
        ),
    )
}

/// Graph's error body. Its shape is documented; apart from `TooManyRequests`
/// the code strings here are this fake's own until answers are recorded from a
/// test tenant, so nothing should match on them.
fn error(status: StatusCode, code: &str, message: &str) -> Response {
    let body: Value = json!({ "error": { "code": code, "message": message } });

    (status, axum::Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get(graph: &FakeGraph, path: &str, token: Option<&str>) -> reqwest::Response {
        let mut request = reqwest::Client::new().get(format!("{}{path}", graph.base_url()));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap()
    }

    #[tokio::test]
    async fn the_signed_in_user_is_returned() {
        let graph = FakeGraph::start().await.unwrap();

        let response = get(&graph, "/me", Some("token")).await;

        assert_eq!(response.status(), 200);
        let user: Value = response.json().await.unwrap();
        assert_eq!(user["userPrincipalName"], "robin@example.test");
        assert_eq!(graph.requests(), ["GET /v1.0/me"]);
    }

    #[tokio::test]
    async fn a_request_without_a_token_is_refused() {
        let graph = FakeGraph::start().await.unwrap();

        assert_eq!(get(&graph, "/me", None).await.status(), 401);
    }

    #[tokio::test]
    async fn faults_are_handed_out_one_a_request_and_then_stop() {
        let graph = FakeGraph::start().await.unwrap();
        graph.fail_next([
            Fault::Throttled {
                retry_after_secs: 7,
            },
            Fault::Unavailable,
        ]);

        let throttled = get(&graph, "/me", Some("token")).await;
        let unavailable = get(&graph, "/me", Some("token")).await;
        let fine = get(&graph, "/me", Some("token")).await;

        assert_eq!(throttled.status(), 429);
        assert_eq!(throttled.headers()["retry-after"], "7");
        assert_eq!(
            throttled.json::<Value>().await.unwrap()["error"]["code"],
            "TooManyRequests"
        );
        assert_eq!(unavailable.status(), 503);
        assert_eq!(fine.status(), 200);
        assert_eq!(graph.requests().len(), 3);
    }

    #[tokio::test]
    async fn something_not_built_says_so() {
        let graph = FakeGraph::start().await.unwrap();

        let response = get(&graph, "/me/calendar/events", Some("token")).await;

        assert_eq!(response.status(), 404);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "fakeGraphNotBuilt");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("/v1.0/me/calendar/events")
        );
    }
}
