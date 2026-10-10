//! A stand-in for Microsoft Graph.
//!
//! It listens on a port of its own and answers the way Graph is documented
//! to, so the client's sync and outbox can be tested without a network or a
//! tenant, and can be made to meet the failures a real one only produces now
//! and then: throttling, and a service that is briefly down.
//!
//! So far it knows one resource, the signed-in user. Each one the client
//! comes to use is added here first.

mod identity;

pub use identity::FakeIdentity;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::Router;
use axum::extract::{Request, State};
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
    /// Every request received, as `GET /v1.0/me`.
    requests: Vec<String>,
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
            .fallback(not_built)
            // Outermost last: a request is logged, then faulted, then checked for a token.
            .layer(middleware::from_fn(require_token))
            .layer(middleware::from_fn_with_state(shared.clone(), inject_fault))
            .layer(middleware::from_fn_with_state(shared.clone(), record))
            .with_state(shared.clone());

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
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
        (!inner.faults.is_empty()).then(|| inner.faults.remove(0))
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
    let has_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("Bearer ") && v.len() > "Bearer ".len());

    if has_token {
        next.run(request).await
    } else {
        error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "A bearer token is required.",
        )
    }
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

        let response = get(&graph, "/me/messages", Some("token")).await;

        assert_eq!(response.status(), 404);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], "fakeGraphNotBuilt");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("/v1.0/me/messages")
        );
    }
}
