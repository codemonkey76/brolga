//! A stand-in for the Microsoft identity platform's v2.0 endpoints: authorize,
//! token and devicecode, as documented (SOURCES.md). It checks what a real one
//! would check, PKCE included, so a client that gets the flow wrong fails here.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::extract::{Form, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::json;
use sha2::{Digest, Sha256};

struct PendingCode {
    challenge: String,
    redirect_uri: String,
    client_id: String,
}

/// The permissions a code, device code or refresh token was asked for
/// with, so the access tokens that come of it carry those and no others.
type Scopes = HashMap<String, String>;

struct Refusal {
    error: String,
    codes: Vec<u32>,
    description: String,
}

#[derive(Default)]
struct Inner {
    issued: u32,
    codes: HashMap<String, PendingCode>,
    refresh_tokens: Vec<String>,
    scopes: Scopes,
    /// Device codes, each with how many more polls to answer "pending".
    device_codes: HashMap<String, u32>,
    pending_polls: u32,
    authorize_refusal: Option<(String, String)>,
    token_refusals: Vec<Refusal>,
    /// Every form posted to the token endpoint, oldest first.
    token_requests: Vec<HashMap<String, String>>,
    /// Every query the authorize endpoint was opened with.
    authorize_requests: Vec<HashMap<String, String>>,
    expires_in: Option<u64>,
    /// Permissions an administrator has given the app for everyone. Every
    /// token carries them, asked for or not, as a real one does.
    admin_granted: Vec<String>,
}

type Shared = Arc<Mutex<Inner>>;

pub struct FakeIdentity {
    address: SocketAddr,
    shared: Shared,
    server: tokio::task::JoinHandle<()>,
}

impl FakeIdentity {
    pub async fn start() -> std::io::Result<Self> {
        let shared = Shared::default();
        let app = Router::new()
            .route("/{tenant}/oauth2/v2.0/authorize", get(authorize))
            .route("/{tenant}/oauth2/v2.0/token", post(token))
            .route("/{tenant}/oauth2/v2.0/devicecode", post(device_code))
            .with_state(shared.clone());

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Ok(Self {
            address,
            shared,
            server,
        })
    }

    /// What to use in place of `https://login.microsoftonline.com`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Send the next visitor to the authorize page back with this error, as
    /// when somebody cancels or their organisation refuses.
    pub fn refuse_next_authorize(&self, error: &str, description: &str) {
        lock(&self.shared).authorize_refusal = Some((error.to_owned(), description.to_owned()));
    }

    /// Answer the next token request with this error.
    pub fn refuse_next_token(&self, error: &str, codes: &[u32], description: &str) {
        lock(&self.shared).token_refusals.push(Refusal {
            error: error.to_owned(),
            codes: codes.to_vec(),
            description: description.to_owned(),
        });
    }

    /// Forget every refresh token issued, as when a password is changed.
    pub fn revoke_refresh_tokens(&self) {
        lock(&self.shared).refresh_tokens.clear();
    }

    /// Have each new device code answer "pending" this many times before the
    /// person is taken to have signed in.
    pub fn device_polls_before_sign_in(&self, polls: u32) {
        lock(&self.shared).pending_polls = polls;
    }

    /// Issue access tokens that last this many seconds, not the usual hour.
    /// An administrator approves these permissions for the whole
    /// organisation: tokens issued from now on carry them.
    pub fn admin_grants(&self, scopes: &[&str]) {
        if let Ok(mut inner) = self.shared.lock() {
            inner.admin_granted = scopes.iter().map(|s| (*s).to_owned()).collect();
        }
    }

    pub fn access_tokens_last(&self, seconds: u64) {
        lock(&self.shared).expires_in = Some(seconds);
    }

    pub fn token_requests(&self) -> Vec<HashMap<String, String>> {
        lock(&self.shared).token_requests.clone()
    }

    pub fn authorize_requests(&self) -> Vec<HashMap<String, String>> {
        lock(&self.shared).authorize_requests.clone()
    }
}

impl Drop for FakeIdentity {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn lock(shared: &Shared) -> MutexGuard<'_, Inner> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The sign-in page. There is nobody to sign in, so it goes straight back to
/// the redirect URI the way the real page does once somebody has.
async fn authorize(
    State(shared): State<Shared>,
    Path(_tenant): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mut inner = lock(&shared);
    inner.authorize_requests.push(query.clone());

    let get = |name: &str| query.get(name).map(String::as_str).unwrap_or_default();
    let (Some(redirect_uri), Some(_client)) = (query.get("redirect_uri"), query.get("client_id"))
    else {
        return (
            StatusCode::BAD_REQUEST,
            "client_id and redirect_uri are required",
        )
            .into_response();
    };
    if get("response_type") != "code"
        || get("code_challenge_method") != "S256"
        || get("code_challenge").is_empty()
    {
        return (
            StatusCode::BAD_REQUEST,
            "an authorization code request with an S256 challenge is required",
        )
            .into_response();
    }

    let state = get("state");
    let location = match inner.authorize_refusal.take() {
        Some((error, description)) => format!(
            "{redirect_uri}/?error={error}&error_description={}&state={state}",
            description.replace(' ', "+")
        ),
        None => {
            inner.issued += 1;
            let code = format!("code-{}", inner.issued);
            inner.scopes.insert(code.clone(), get("scope").to_owned());
            inner.codes.insert(
                code.clone(),
                PendingCode {
                    challenge: get("code_challenge").to_owned(),
                    redirect_uri: redirect_uri.clone(),
                    client_id: get("client_id").to_owned(),
                },
            );
            format!("{redirect_uri}/?code={code}&state={state}")
        }
    };

    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}

async fn token(
    State(shared): State<Shared>,
    Path(_tenant): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mut inner = lock(&shared);
    inner.token_requests.push(form.clone());

    if !inner.token_refusals.is_empty() {
        let refusal = inner.token_refusals.remove(0);
        return refuse(&refusal.error, &refusal.codes, &refusal.description);
    }

    let get = |name: &str| form.get(name).map(String::as_str).unwrap_or_default();

    match get("grant_type") {
        "authorization_code" => {
            let Some(pending) = inner.codes.remove(get("code")) else {
                return refuse(
                    "invalid_grant",
                    &[70008],
                    "The code is not one this server issued, or was used already.",
                );
            };
            let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(get("code_verifier").as_bytes()));
            if challenge != pending.challenge
                || get("redirect_uri") != pending.redirect_uri
                || get("client_id") != pending.client_id
            {
                return refuse(
                    "invalid_grant",
                    &[501_481],
                    "The code verifier, redirect URI or client does not match the request.",
                );
            }
            issue(&mut inner, get("code"))
        }
        "refresh_token" => {
            if !inner
                .refresh_tokens
                .iter()
                .any(|t| t == get("refresh_token"))
            {
                return refuse(
                    "invalid_grant",
                    &[50173],
                    "The provided grant has expired due to it being revoked.",
                );
            }
            issue(&mut inner, get("refresh_token"))
        }
        "urn:ietf:params:oauth:grant-type:device_code" => {
            match inner.device_codes.get_mut(get("device_code")) {
                None => refuse(
                    "bad_verification_code",
                    &[],
                    "The device code was not recognised.",
                ),
                Some(0) => {
                    inner.device_codes.remove(get("device_code"));
                    issue(&mut inner, get("device_code"))
                }
                Some(polls) => {
                    *polls -= 1;
                    refuse(
                        "authorization_pending",
                        &[70016],
                        "The user has not finished signing in.",
                    )
                }
            }
        }
        other => refuse(
            "unsupported_grant_type",
            &[],
            &format!("`{other}` is not a grant this server knows."),
        ),
    }
}

async fn device_code(
    State(shared): State<Shared>,
    Path(_tenant): Path<String>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    if !form.contains_key("client_id") || !form.contains_key("scope") {
        return refuse("invalid_request", &[], "client_id and scope are required.");
    }

    let mut inner = lock(&shared);
    inner.issued += 1;
    let device_code = format!("device-{}", inner.issued);
    let polls = inner.pending_polls;
    inner.device_codes.insert(device_code.clone(), polls);
    let asked = form.get("scope").cloned().unwrap_or_default();
    inner.scopes.insert(device_code.clone(), asked);

    Json(json!({
        "device_code": device_code,
        "user_code": "ABCD1234",
        "verification_uri": "https://login.example.test/device",
        "expires_in": 900,
        "interval": 1,
        "message": "To sign in, open https://login.example.test/device and enter the code ABCD1234."
    }))
    .into_response()
}

/// Issue tokens for what `from` (a code, device code or refresh token) was
/// asked for with. The access token carries its permissions after a dot, as
/// a real one carries them in its claims, and the fake Graph server holds
/// each request to them.
fn issue(inner: &mut Inner, from: &str) -> Response {
    inner.issued += 1;
    let refresh = format!("refresh-{}", inner.issued);
    inner.refresh_tokens.push(refresh.clone());
    let scope = inner.scopes.get(from).cloned().unwrap_or_default();
    inner.scopes.insert(refresh.clone(), scope.clone());
    let admin_granted = inner.admin_granted.clone();
    let granted: Vec<&str> = scope
        .split_whitespace()
        .filter(|s| !["openid", "profile", "offline_access"].contains(s))
        .chain(admin_granted.iter().map(String::as_str))
        .collect();

    let claims = json!({
        "tid": "00000000-0000-4000-8000-0000000000aa",
        "oid": "00000000-0000-4000-8000-000000000001",
        "preferred_username": "robin@example.test",
        "name": "Robin Example"
    });
    let id_token = format!(
        "e30.{}.unsigned",
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );

    Json(json!({
        "token_type": "Bearer",
        "scope": granted.join(" "),
        "expires_in": inner.expires_in.unwrap_or(3600),
        "access_token": format!("access-{}.{}", inner.issued, granted.join(",")),
        "refresh_token": refresh,
        "id_token": id_token
    }))
    .into_response()
}

fn refuse(error: &str, codes: &[u32], description: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error, "error_description": description, "error_codes": codes })),
    )
        .into_response()
}
