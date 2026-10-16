//! A stand-in MCP server, as a service's would be, for tests that run
//! offline: the older Streamable HTTP protocol (`initialize` first, as the
//! 2025-11-25 revision has it, which is what a server built on most of
//! today's libraries speaks), `tools/list` and `tools/call`, behind either
//! a fixed access token or OAuth from an authorization server of its own
//! (protected resource metadata, authorization server metadata, dynamic
//! client registration, PKCE with S256, refresh tokens).

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::Router;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Default)]
struct Inner {
    base: String,
    tools: Vec<Value>,
    /// Access tokens that are good now.
    tokens: HashSet<String>,
    refresh_tokens: HashSet<String>,
    /// Codes given out, with the PKCE challenge each was given for.
    codes: HashMap<String, String>,
    next: u32,
    calls: Vec<(String, Value)>,
    registered: u32,
    refreshed: u32,
    /// Give out tokens that are good for so many seconds.
    lifetime: Option<u64>,
    /// What a tool answers, as JSON, in place of the usual line.
    answers: HashMap<String, Value>,
    /// Tools that say they failed, with what they say.
    failing: HashMap<String, String>,
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Inner>>);

fn lock(shared: &Shared) -> MutexGuard<'_, Inner> {
    shared
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct FakeMcp {
    address: SocketAddr,
    shared: Shared,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for FakeMcp {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl FakeMcp {
    /// A server with two tools, like a helpdesk's.
    pub async fn start() -> std::io::Result<Self> {
        let shared = Shared::default();
        let app = Router::new()
            .route("/mcp", post(mcp).get(no_stream).delete(no_stream))
            .route("/.well-known/oauth-protected-resource/mcp", get(resource))
            .route("/.well-known/oauth-protected-resource", get(resource))
            .route(
                "/.well-known/oauth-authorization-server",
                get(authorization_server),
            )
            .route("/register", post(register))
            .route("/oauth2/authorize", get(authorize))
            .route("/token", post(token))
            .with_state(shared.clone());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        {
            let mut inner = lock(&shared);
            inner.base = format!("http://{address}");
            inner.tools = vec![
                json!({
                    "name": "create_ticket", "title": "Create a ticket",
                    "description": "Open a new ticket.",
                    "inputSchema": { "type": "object", "properties": {
                        "client_id": { "type": "integer" }, "subject": { "type": "string" }, "description": { "type": "string" }
                    }, "required": ["client_id", "subject", "description"] }
                }),
                json!({
                    "name": "list_tickets", "description": "List the tickets.",
                    "inputSchema": { "type": "object", "properties": { "search": { "type": "string" } } }
                }),
            ];
        }
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Ok(Self {
            address,
            shared,
            server,
        })
    }

    /// The MCP endpoint.
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.address)
    }

    /// Take this as an access token, as a personal access token is.
    pub fn accept_token(&self, token: &str) {
        lock(&self.shared).tokens.insert(token.to_owned());
    }

    /// Stop taking every access token given out, as when they expire.
    pub fn expire_tokens(&self) {
        lock(&self.shared).tokens.clear();
    }

    /// Offer these tools in place of the two it starts with.
    pub fn set_tools(&self, tools: Vec<Value>) {
        lock(&self.shared).tools = tools;
    }

    /// The four tools a helpdesk has for making a ticket for whoever wrote:
    /// find a client and its contacts, make a client, make a contact, make
    /// a ticket.
    pub fn helpdesk_tools(&self) {
        self.set_tools(vec![
            json!({ "name": "list_clients", "description": "List clients with their contacts.",
                "inputSchema": { "type": "object", "properties": { "contact_email": { "type": "string" }, "search": { "type": "string" } } } }),
            json!({ "name": "create_client", "description": "Make a client.",
                "inputSchema": { "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] } }),
            json!({ "name": "create_client_user", "description": "Make a contact of a client.",
                "inputSchema": { "type": "object", "properties": {
                    "client_id": { "type": "integer" }, "name": { "type": "string" }, "email": { "type": "string" }
                }, "required": ["client_id", "name", "email"] } }),
            json!({ "name": "create_ticket", "description": "Open a ticket.",
                "inputSchema": { "type": "object", "properties": {
                    "client_id": { "type": "integer" }, "user_id": { "type": "integer" }, "subject": { "type": "string" },
                    "description": { "type": "string" }, "assigned_agent_id": { "type": "integer" }
                }, "required": ["client_id", "user_id", "subject", "description"] } }),
        ]);
    }

    /// Have a tool answer with this, as JSON text.
    pub fn answer(&self, tool: &str, answer: Value) {
        lock(&self.shared).answers.insert(tool.to_owned(), answer);
    }

    /// Have a tool say it failed, with this.
    pub fn fail(&self, tool: &str, said: &str) {
        lock(&self.shared)
            .failing
            .insert(tool.to_owned(), said.to_owned());
    }

    /// The tool calls made: name and arguments.
    pub fn calls(&self) -> Vec<(String, Value)> {
        lock(&self.shared).calls.clone()
    }

    /// Give out tokens good for only a second from now on.
    pub fn short_lived_tokens(&self) {
        lock(&self.shared).lifetime = Some(1);
    }

    /// How many clients registered themselves.
    pub fn registered(&self) -> u32 {
        lock(&self.shared).registered
    }

    /// How many times a token was refreshed.
    pub fn refreshed(&self) -> u32 {
        lock(&self.shared).refreshed
    }
}

fn unauthorized(base: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(
            header::WWW_AUTHENTICATE,
            format!("Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\""),
        )],
        "",
    )
        .into_response()
}

async fn no_stream() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

async fn mcp(
    State(shared): State<Shared>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut inner = lock(&shared);
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !inner.tokens.contains(bearer) {
        return unauthorized(&inner.base);
    }
    let id = body["id"].clone();
    let method = body["method"].as_str().unwrap_or_default();
    let answer = |result: Value| {
        axum::Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
    };
    match method {
        "initialize" => {
            let mut response = answer(json!({
                "protocolVersion": "2025-11-25",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "Fake helpdesk", "version": "1.0.0" }
            }));
            if let Ok(session) = "session-1".parse() {
                response.headers_mut().insert("mcp-session-id", session);
            }
            response
        }
        m if m.starts_with("notifications/") => StatusCode::ACCEPTED.into_response(),
        "ping" => answer(json!({})),
        "tools/list" => answer(json!({ "tools": inner.tools })),
        "tools/call" => {
            let name = body["params"]["name"].as_str().unwrap_or_default().to_owned();
            let arguments = body["params"]["arguments"].clone();
            inner.calls.push((name.clone(), arguments));
            if let Some(said) = inner.failing.get(&name) {
                return answer(json!({ "content": [{ "type": "text", "text": said }], "isError": true }));
            }
            let text = match inner.answers.get(&name) {
                Some(given) => given.to_string(),
                None => format!("{name} done: TKT-01234"),
            };
            answer(json!({ "content": [{ "type": "text", "text": text }], "isError": false }))
        }
        // An older server has no `server/discover`.
        _ => axum::Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "Method not found" } }))
            .into_response(),
    }
}

async fn resource(State(shared): State<Shared>) -> Response {
    let base = lock(&shared).base.clone();
    axum::Json(json!({ "resource": format!("{base}/mcp"), "authorization_servers": [base] }))
        .into_response()
}

async fn authorization_server(State(shared): State<Shared>) -> Response {
    let base = lock(&shared).base.clone();
    axum::Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth2/authorize"),
        "token_endpoint": format!("{base}/token"),
        "registration_endpoint": format!("{base}/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"]
    }))
    .into_response()
}

async fn register(State(shared): State<Shared>, axum::Json(body): axum::Json<Value>) -> Response {
    let mut inner = lock(&shared);
    inner.registered += 1;
    let mut made = body.clone();
    made["client_id"] = json!(format!("client-{}", inner.registered));
    made["token_endpoint_auth_method"] = json!("none");
    (StatusCode::CREATED, axum::Json(made)).into_response()
}

/// The person agrees at once, and the browser is sent back with a code.
async fn authorize(
    State(shared): State<Shared>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mut inner = lock(&shared);
    if query.get("code_challenge_method").map(String::as_str) != Some("S256")
        || !query.contains_key("resource")
    {
        return (
            StatusCode::BAD_REQUEST,
            "PKCE with S256 and a resource are required",
        )
            .into_response();
    }
    inner.next += 1;
    let code = format!("code-{}", inner.next);
    inner.codes.insert(
        code.clone(),
        query.get("code_challenge").cloned().unwrap_or_default(),
    );
    let redirect = format!(
        "{}?code={code}&state={}&iss={}",
        query.get("redirect_uri").cloned().unwrap_or_default(),
        query.get("state").cloned().unwrap_or_default(),
        inner.base
    );
    (StatusCode::FOUND, [(header::LOCATION, redirect)]).into_response()
}

async fn token(
    State(shared): State<Shared>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mut inner = lock(&shared);
    let refused = |error: &str| {
        (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({ "error": error })),
        )
            .into_response()
    };
    match form.get("grant_type").map(String::as_str) {
        Some("authorization_code") => {
            let Some(challenge) = form.get("code").and_then(|code| inner.codes.remove(code)) else {
                return refused("invalid_grant");
            };
            let verifier = form.get("code_verifier").cloned().unwrap_or_default();
            let made = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(verifier.as_bytes()));
            if made != challenge {
                return refused("invalid_grant");
            }
        }
        Some("refresh_token") => {
            let given = form.get("refresh_token").cloned().unwrap_or_default();
            if !inner.refresh_tokens.remove(&given) {
                return refused("invalid_grant");
            }
            inner.refreshed += 1;
        }
        _ => return refused("unsupported_grant_type"),
    }
    inner.next += 1;
    let access = format!("at-{}", inner.next);
    let refresh = format!("rt-{}", inner.next);
    inner.tokens.insert(access.clone());
    inner.refresh_tokens.insert(refresh.clone());
    let lifetime = inner.lifetime.unwrap_or(3600);
    axum::Json(json!({ "access_token": access, "token_type": "Bearer", "expires_in": lifetime, "refresh_token": refresh }))
        .into_response()
}
