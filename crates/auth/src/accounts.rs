use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use brolga_store::{Account, GlobalDb, StoreError};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::{AuthError, Problem, Registration, SecretError, SecretStore, TokenClient, Tokens};

/// An access token this close to expiring is replaced before it is handed
/// out, so a request made with it does not fail part-way.
const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("There is no account `{0}`.")]
    Unknown(String),
    /// The saved sign-in no longer works. Everything already on this
    /// computer is still there to read.
    #[error("{username} needs to sign in again. {}", .problem.explain())]
    NeedsSignIn { username: String, problem: Problem },
    #[error("The sign-in did not say who signed in, so the account cannot be added.")]
    NoIdentity,
    #[error("The sign-in gave nothing to stay signed in with, so the account cannot be added.")]
    NoRefreshToken,
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Secrets(#[from] SecretError),
}

/// The signed-in accounts, and the tokens that keep them signed in.
///
/// The list is in the global database. Each account's refresh token is in
/// the secret store. Access tokens are here in memory and nowhere else.
pub struct Accounts {
    db: Arc<GlobalDb>,
    secrets: Arc<dyn SecretStore>,
    client: TokenClient,
    /// Held across a refresh, so two callers never spend the same refresh
    /// token at once.
    live: Mutex<HashMap<String, Tokens>>,
}

impl Accounts {
    pub fn new(db: Arc<GlobalDb>, secrets: Arc<dyn SecretStore>, client: TokenClient) -> Self {
        Self {
            db,
            secrets,
            client,
            live: Mutex::new(HashMap::new()),
        }
    }

    pub fn client(&self) -> &TokenClient {
        &self.client
    }

    /// The client ID the last successful sign-in on this computer used.
    pub fn remembered_client_id(&self) -> Option<String> {
        self.db.remembered_client_id().ok().flatten()
    }

    pub fn list(&self) -> Result<Vec<Account>, AccountError> {
        Ok(self.db.accounts()?)
    }

    /// Keep what a finished sign-in gave: the account in the list, its
    /// refresh token in the secret store. Signing in again to an account
    /// already here brings it up to date.
    pub async fn add(
        &self,
        registration: &Registration,
        tokens: Tokens,
    ) -> Result<Account, AccountError> {
        let identity = tokens.identity.clone().ok_or(AccountError::NoIdentity)?;
        let refresh_token = tokens
            .refresh_token
            .clone()
            .ok_or(AccountError::NoRefreshToken)?;
        let id = format!("{}.{}", identity.tenant_id, identity.object_id);

        // The secret first: an account in the list with no way to stay
        // signed in is worse than a stray secret no account points at.
        self.secrets.set(&secret_key(&id), &refresh_token)?;
        let account = self.db.save_account(&Account {
            id: id.clone(),
            tenant_id: identity.tenant_id,
            object_id: identity.object_id,
            username: identity.username,
            display_name: identity.name,
            client_id: registration.client_id.clone(),
            tenant: registration.tenant.clone(),
            needs_sign_in: false,
            added_at: String::new(),
        })?;
        self.live.lock().await.insert(id, tokens);

        Ok(account)
    }

    /// Forget an account and its saved sign-in. True if there was one.
    pub async fn remove(&self, id: &str) -> Result<bool, AccountError> {
        let removed = self.db.remove_account(id)?;
        self.secrets.delete(&secret_key(id))?;
        self.live.lock().await.remove(id);

        Ok(removed)
    }

    /// An access token for this account that is good for at least the next
    /// few minutes, refreshed first if need be.
    pub async fn access_token(&self, id: &str) -> Result<String, AccountError> {
        let mut live = self.live.lock().await;

        if let Some(tokens) = live.get(id)
            && tokens.expires_at.saturating_duration_since(Instant::now()) > REFRESH_MARGIN
        {
            return Ok(tokens.access_token.clone());
        }

        let account = self
            .db
            .account(id)?
            .ok_or_else(|| AccountError::Unknown(id.to_owned()))?;
        let needs_sign_in = |problem| AccountError::NeedsSignIn {
            username: account.username.clone(),
            problem,
        };
        if account.needs_sign_in {
            return Err(needs_sign_in(Problem::SignInExpired));
        }

        let Some(refresh_token) = self.secrets.get(&secret_key(id))? else {
            // The secret store was cleared from under us.
            self.db.set_needs_sign_in(id, true)?;
            return Err(needs_sign_in(Problem::SignInExpired));
        };

        let registration = Registration {
            client_id: account.client_id.clone(),
            tenant: account.tenant.clone(),
        };
        let mut tokens = match self.client.refresh(&registration, &refresh_token).await {
            Ok(tokens) => tokens,
            // The identity platform said no: the saved sign-in is done for,
            // whatever the reason. Being offline is not that, and leaves the
            // account as it is for when the network comes back.
            Err(AuthError::Refused { problem, .. }) => {
                live.remove(id);
                self.db.set_needs_sign_in(id, true)?;
                return Err(needs_sign_in(problem));
            }
            Err(other) => return Err(other.into()),
        };

        // A refresh answers with a new refresh token that replaces the old.
        match &tokens.refresh_token {
            Some(new) => self.secrets.set(&secret_key(id), new)?,
            None => tokens.refresh_token = Some(refresh_token),
        }

        let access_token = tokens.access_token.clone();
        live.insert(id.to_owned(), tokens);

        Ok(access_token)
    }
}

fn secret_key(account_id: &str) -> String {
    format!("refresh-token.{account_id}")
}
