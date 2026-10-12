use std::fmt;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::{AuthError, Authority, Registration};

/// Who signed in, from the ID token's claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// The organisation's immutable ID (`tid`).
    pub tenant_id: String,
    /// The person's immutable ID in that organisation (`oid`).
    pub object_id: String,
    /// Their sign-in name, usually an email address (`preferred_username`). It can change.
    pub username: String,
    pub name: Option<String>,
}

/// What a successful sign-in or refresh gives.
#[derive(Clone)]
pub struct Tokens {
    pub access_token: String,
    /// When the access token stops working.
    pub expires_at: Instant,
    /// Replaces the one before it. Absent only if `offline_access` was not granted.
    pub refresh_token: Option<String>,
    pub identity: Option<Identity>,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"[redacted]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[redacted]"),
            )
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct TokenBody {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
    #[serde(default)]
    error_description: String,
    #[serde(default)]
    error_codes: Vec<u32>,
}

#[derive(Deserialize)]
struct IdClaims {
    tid: String,
    oid: String,
    preferred_username: String,
    name: Option<String>,
}

/// The device code grant's opening answer.
#[derive(Deserialize)]
pub(crate) struct DeviceCodeBody {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
    pub message: String,
}

/// The longest anything the identity platform gives out is taken to last,
/// whatever it says. An access token lasts about an hour and a device code
/// a quarter of one; a number of seconds beyond counting, added to the
/// time, would otherwise stop the program.
const LONGEST_LIFE: Duration = Duration::from_secs(24 * 60 * 60);

/// The largest answer read from a token endpoint: each is a few tokens.
const MAX_ANSWER: usize = 1024 * 1024;

/// How long something lasts that is said to last so many seconds.
pub(crate) fn life(seconds: u64) -> Duration {
    Duration::from_secs(seconds).min(LONGEST_LIFE)
}

/// Talks to the identity platform's token endpoints.
#[derive(Clone)]
pub struct TokenClient {
    http: reqwest::Client,
    authority: Authority,
}

impl TokenClient {
    pub fn new(authority: Authority) -> Result<Self, AuthError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            // A token endpoint has no business sending us anywhere else.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(AuthError::Network)?;

        Ok(Self { http, authority })
    }

    pub fn authority(&self) -> &Authority {
        &self.authority
    }

    pub(crate) async fn exchange_code(
        &self,
        registration: &Registration,
        code: &str,
        redirect_uri: &str,
        verifier: &str,
    ) -> Result<Tokens, AuthError> {
        self.grant(
            registration,
            &[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
            ],
        )
        .await
    }

    /// Trade a refresh token for a new access token. The answer carries a new
    /// refresh token, which must replace the old one.
    pub async fn refresh(
        &self,
        registration: &Registration,
        refresh_token: &str,
    ) -> Result<Tokens, AuthError> {
        self.grant(
            registration,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ],
        )
        .await
    }

    pub(crate) async fn device_grant(
        &self,
        registration: &Registration,
        device_code: &str,
    ) -> Result<Tokens, AuthError> {
        self.grant(
            registration,
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", device_code),
            ],
        )
        .await
    }

    pub(crate) async fn device_code(
        &self,
        registration: &Registration,
        scopes: &[&str],
    ) -> Result<DeviceCodeBody, AuthError> {
        let scope = scopes.join(" ");
        let response = self
            .http
            .post(self.authority.device_code(&registration.tenant))
            .form(&[
                ("client_id", registration.client_id.as_str()),
                ("scope", scope.as_str()),
            ])
            .send()
            .await
            .map_err(AuthError::Network)?;

        read(response).await
    }

    async fn grant(
        &self,
        registration: &Registration,
        fields: &[(&str, &str)],
    ) -> Result<Tokens, AuthError> {
        let mut form = vec![("client_id", registration.client_id.as_str())];
        form.extend_from_slice(fields);

        let response = self
            .http
            .post(self.authority.token(&registration.tenant))
            .form(&form)
            .send()
            .await
            .map_err(AuthError::Network)?;
        let body: TokenBody = read(response).await?;

        Ok(Tokens {
            access_token: body.access_token,
            expires_at: Instant::now() + life(body.expires_in),
            refresh_token: body.refresh_token,
            identity: body.id_token.as_deref().map(identity).transpose()?,
        })
    }
}

/// A 2xx answer is the body asked for; anything else is the platform's error body.
async fn read<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T, AuthError> {
    let status = response.status();
    // A piece at a time, and given up on once it is larger than any
    // answer could be: a server need not say how much it is about to send.
    let too_large = || AuthError::Protocol("an answer too large to be one".to_owned());
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ANSWER as u64)
    {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    while let Some(piece) = response.chunk().await.map_err(AuthError::Network)? {
        if bytes.len() + piece.len() > MAX_ANSWER {
            return Err(too_large());
        }
        bytes.extend_from_slice(&piece);
    }

    if status.is_success() {
        return serde_json::from_slice(&bytes).map_err(|e| AuthError::Protocol(e.to_string()));
    }

    match serde_json::from_slice::<ErrorBody>(&bytes) {
        Ok(body) => Err(AuthError::refused(
            &body.error,
            body.error_codes,
            body.error_description,
        )),
        Err(_) => Err(AuthError::Protocol(format!(
            "HTTP {status} with no error it could explain"
        ))),
    }
}

/// Read who signed in from an ID token.
///
/// The signature is not checked. The token came straight from the token
/// endpoint over TLS, which OpenID Connect Core section 3.1.3.7 allows in
/// place of checking it, and the claims are used to label the account, never
/// to decide what anyone may do.
fn identity(id_token: &str) -> Result<Identity, AuthError> {
    let payload = id_token
        .split('.')
        .nth(1)
        .ok_or_else(|| AuthError::Protocol("the ID token is not a JWT".to_owned()))?;
    let json = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|e| AuthError::Protocol(format!("the ID token is not base64url: {e}")))?;
    let claims: IdClaims = serde_json::from_slice(&json)
        .map_err(|e| AuthError::Protocol(format!("the ID token lacks a claim: {e}")))?;

    Ok(Identity {
        tenant_id: claims.tid,
        object_id: claims.oid,
        username: claims.preferred_username,
        name: claims.name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: serde_json::Value) -> String {
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(claims.to_string()))
    }

    #[test]
    fn who_signed_in_is_read_from_the_id_token() {
        let token = jwt(serde_json::json!({
            "tid": "t-1", "oid": "o-1", "preferred_username": "robin@example.test", "name": "Robin Example", "aud": "x"
        }));

        assert_eq!(
            identity(&token).unwrap(),
            Identity {
                tenant_id: "t-1".to_owned(),
                object_id: "o-1".to_owned(),
                username: "robin@example.test".to_owned(),
                name: Some("Robin Example".to_owned()),
            }
        );
    }

    #[test]
    fn an_id_token_that_is_not_one_is_an_error_not_a_panic() {
        for bad in [
            "",
            "one-part",
            "a.!!!.c",
            &jwt(serde_json::json!({ "tid": "t" })),
        ] {
            assert!(
                matches!(identity(bad), Err(AuthError::Protocol(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn nothing_is_taken_to_last_longer_than_a_day() {
        assert_eq!(life(3600), Duration::from_secs(3600));
        assert_eq!(life(u64::MAX), LONGEST_LIFE);
        // What stopped the program: the time, and that many seconds more.
        let _ = Instant::now() + life(u64::MAX);
    }

    #[test]
    fn tokens_do_not_print() {
        let tokens = Tokens {
            access_token: "ACCESS-SECRET".to_owned(),
            expires_at: Instant::now(),
            refresh_token: Some("REFRESH-SECRET".to_owned()),
            identity: None,
        };

        let printed = format!("{tokens:?}");

        assert!(!printed.contains("SECRET"), "{printed}");
    }
}
