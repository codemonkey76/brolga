//! Tools on other services, reached as an MCP client
//! (docs/spec-mail-instructions.md, "Outside tools").
//!
//! The person adds a server by its address and signs in to it, either
//! through their browser (OAuth, as the MCP authorization specification
//! describes, which rmcp carries out) or with an access token the service
//! gave them. What it is signed in with is kept in the system's secret
//! store and never given out. Brolga then reads which tools the server has.
//! Calling them is for the instructions that name them, in the next part.

mod loopback;

use std::sync::Arc;
use std::time::Duration;

use brolga_auth::SecretStore;
use rmcp::RoleClient;
use rmcp::model::{ClientCapabilities, ClientConfig, Implementation, ProtocolVersion};
use rmcp::service::{ClientLifecycleMode, ClientServiceExt, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::auth::{
    AuthClient, AuthError, AuthorizationManager, AuthorizationRequest, CredentialStore, OAuthState,
    StoredCredentials,
};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

pub use loopback::Loopback;

/// A server the person has added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    pub id: String,
    pub name: String,
    /// Where it is: its MCP endpoint.
    pub url: String,
}

/// A tool a server has, as it describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolsError {
    #[error("{0}")]
    Address(String),
    #[error("Brolga is not signed in to {0}. Sign in to it again in Settings, Outside tools.")]
    NotSignedIn(String),
    #[error("{0} could not be reached: {1}")]
    Unreached(String, String),
    #[error("Signing in to {0} did not work: {1}")]
    SignIn(String, String),
    #[error("The secret store could not be used: {0}")]
    Secrets(String),
}

impl ToolsError {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Unreached(..))
    }
}

/// A server's address, if mail may be sent to it: anywhere over `https`,
/// or `http` on this computer only.
pub fn checked_address(address: &str) -> Result<Url, ToolsError> {
    let url = Url::parse(address.trim())
        .map_err(|_| ToolsError::Address(format!("`{address}` is not a web address.")))?;
    let local = match url.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => {
            return Err(ToolsError::Address(format!(
                "`{address}` names no computer."
            )));
        }
    };
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        "http" => {
            return Err(ToolsError::Address(
                "A server on another computer must be reached over https.".to_owned(),
            ));
        }
        _ => {
            return Err(ToolsError::Address(format!(
                "`{address}` is not an http or https address."
            )));
        }
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(ToolsError::Address(
            "Give the server's address without a name, password or # in it.".to_owned(),
        ));
    }

    Ok(url)
}

/// Where a server's sign-in is kept in the secret store.
fn token_key(server: &Server) -> String {
    format!("mcp:{}:token", server.id)
}
fn oauth_key(server: &Server) -> String {
    format!("mcp:{}:oauth", server.id)
}

/// How Brolga is signed in to a server, if it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignedIn {
    Token,
    Browser,
}

pub fn signed_in(secrets: &dyn SecretStore, server: &Server) -> Option<SignedIn> {
    let held = |key: String| matches!(secrets.get(&key), Ok(Some(v)) if !v.is_empty());
    if held(oauth_key(server)) {
        Some(SignedIn::Browser)
    } else if held(token_key(server)) {
        Some(SignedIn::Token)
    } else {
        None
    }
}

/// Keep an access token the service gave the person, in place of any
/// sign-in there was.
pub fn save_token(
    secrets: &dyn SecretStore,
    server: &Server,
    token: &str,
) -> Result<(), ToolsError> {
    let secret = |e: brolga_auth::SecretError| ToolsError::Secrets(e.to_string());
    secrets.delete(&oauth_key(server)).map_err(secret)?;
    let token = token.trim().trim_start_matches("Bearer ").trim();
    if token.is_empty() {
        return secrets.delete(&token_key(server)).map_err(secret);
    }
    secrets.set(&token_key(server), token).map_err(secret)
}

/// Forget every way Brolga was signed in to a server.
pub fn forget(secrets: &dyn SecretStore, server: &Server) -> Result<(), ToolsError> {
    let secret = |e: brolga_auth::SecretError| ToolsError::Secrets(e.to_string());
    secrets.delete(&oauth_key(server)).map_err(secret)?;
    secrets.delete(&token_key(server)).map_err(secret)
}

/// OAuth credentials kept in the secret store, for rmcp to read, refresh
/// and write back.
struct SecretCredentials {
    secrets: Arc<dyn SecretStore>,
    key: String,
}

#[async_trait::async_trait]
impl CredentialStore for SecretCredentials {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match self.secrets.get(&self.key) {
            Ok(Some(held)) if !held.is_empty() => serde_json::from_str(&held)
                .map(Some)
                .map_err(|e| AuthError::InternalError(e.to_string())),
            Ok(_) => Ok(None),
            Err(e) => Err(AuthError::InternalError(e.to_string())),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let held = serde_json::to_string(&credentials)
            .map_err(|e| AuthError::InternalError(e.to_string()))?;
        self.secrets
            .set(&self.key, &held)
            .map_err(|e| AuthError::InternalError(e.to_string()))
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.secrets
            .delete(&self.key)
            .map_err(|e| AuthError::InternalError(e.to_string()))
    }
}

/// Who Brolga says it is to a server.
fn client_info() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("Brolga", env!("CARGO_PKG_VERSION")),
    )
}

/// Sign in through the person's browser. `open` hands the browser the page
/// to go to; this waits for the server to send the browser back here, for
/// up to `wait`.
pub async fn sign_in(
    secrets: Arc<dyn SecretStore>,
    server: &Server,
    open: impl Fn(&str) -> std::io::Result<()>,
    wait: Duration,
) -> Result<(), ToolsError> {
    let url = checked_address(&server.url)?;
    let failed = |e: AuthError| ToolsError::SignIn(server.name.clone(), e.to_string());
    let loopback = Loopback::open()
        .await
        .map_err(|e| ToolsError::SignIn(server.name.clone(), e))?;
    let mut manager = AuthorizationManager::new(url.as_str())
        .await
        .map_err(failed)?;
    manager.set_credential_store(SecretCredentials {
        secrets: secrets.clone(),
        key: oauth_key(server),
    });
    let mut state = OAuthState::Unauthorized(manager);
    state
        .start_authorization(
            AuthorizationRequest::new(loopback.redirect_uri())
                .with_client_name("Brolga")
                .with_application_type("native"),
        )
        .await
        .map_err(failed)?;
    let page = state.get_authorization_url().await.map_err(failed)?;
    open(&page).map_err(|e| {
        ToolsError::SignIn(
            server.name.clone(),
            format!("the browser could not be opened: {e}"),
        )
    })?;
    let back = loopback
        .wait(wait)
        .await
        .map_err(|e| ToolsError::SignIn(server.name.clone(), e))?;
    state
        .handle_callback_with_issuer(&back.code, &back.state, back.issuer.as_deref())
        .await
        .map_err(failed)?;
    // A token given before is no longer what is signed in with.
    let _ = secrets.delete(&token_key(server));

    Ok(())
}

/// A connection to a server, signed in.
pub type Connection = RunningService<RoleClient, ClientConfig>;

/// Connect to a server as Brolga is signed in to it. Both kinds of MCP
/// server are spoken to: those that answer `server/discover`, and older
/// ones that want `initialize` first.
pub async fn connect(
    secrets: Arc<dyn SecretStore>,
    server: &Server,
) -> Result<Connection, ToolsError> {
    let url = checked_address(&server.url)?;
    let unreached = |e: String| ToolsError::Unreached(server.name.clone(), e);
    let lifecycle = ClientLifecycleMode::Auto {
        preferred_versions: vec![ProtocolVersion::LATEST],
        legacy_version: Some(ProtocolVersion::LATEST_WITH_INITIALIZE),
    };
    match signed_in(secrets.as_ref(), server) {
        Some(SignedIn::Browser) => {
            let mut manager = AuthorizationManager::new(url.as_str())
                .await
                .map_err(|e| unreached(e.to_string()))?;
            manager.set_credential_store(SecretCredentials {
                secrets: secrets.clone(),
                key: oauth_key(server),
            });
            if !manager
                .initialize_from_store()
                .await
                .map_err(|e| unreached(e.to_string()))?
            {
                return Err(ToolsError::NotSignedIn(server.name.clone()));
            }
            let client = AuthClient::new(reqwest::Client::new(), manager);
            let transport = StreamableHttpClientTransport::with_client(
                client,
                StreamableHttpClientTransportConfig::with_uri(url.as_str()),
            );
            client_info()
                .serve_with_lifecycle(transport, lifecycle)
                .await
                .map_err(|e| unreached(e.to_string()))
        }
        Some(SignedIn::Token) => {
            let token = secrets
                .get(&token_key(server))
                .map_err(|e| ToolsError::Secrets(e.to_string()))?
                .unwrap_or_default();
            let transport = StreamableHttpClientTransport::from_config(
                StreamableHttpClientTransportConfig::with_uri(url.as_str()).auth_header(token),
            );
            client_info()
                .serve_with_lifecycle(transport, lifecycle)
                .await
                .map_err(|e| unreached(e.to_string()))
        }
        None => Err(ToolsError::NotSignedIn(server.name.clone())),
    }
}

/// The tools a server has.
pub async fn tools(
    secrets: Arc<dyn SecretStore>,
    server: &Server,
) -> Result<Vec<ToolInfo>, ToolsError> {
    let connection = connect(secrets, server).await?;
    let listed = connection
        .list_all_tools()
        .await
        .map_err(|e| ToolsError::Unreached(server.name.clone(), e.to_string()));
    let _ = connection.cancel().await;

    Ok(listed?
        .into_iter()
        .map(|tool| ToolInfo {
            name: tool.name.to_string(),
            title: tool.title.clone(),
            description: tool.description.as_deref().map(str::to_owned),
            input_schema: Value::Object((*tool.input_schema).clone()),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_server_is_reached_over_https_or_on_this_computer() {
        assert!(checked_address("https://helpdesk.example.com/mcp/helpdesk").is_ok());
        assert!(checked_address("http://127.0.0.1:8080/mcp").is_ok());
        assert!(checked_address("http://helpdesk.example.com/mcp").is_err());
        assert!(checked_address("https://me:pw@helpdesk.example.com/mcp").is_err());
        assert!(checked_address("ftp://localhost/mcp").is_err());
    }
}
