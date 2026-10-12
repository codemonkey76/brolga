//! The commands for adding, listing and removing accounts.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use brolga_auth::{Accounts, BrowserSignIn, DeviceSignIn, Registration, SIGN_IN_SCOPES};
use brolga_commands::{CommandError, CommandKind, CommandSpec, Registry, RegistryError};
use brolga_store::Account;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::OpenBrowser;

/// How long a person has to finish signing in in their browser.
const BROWSER_WAIT: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
pub(crate) struct DeviceSignIns {
    next: AtomicU64,
    waiting: Mutex<HashMap<String, (DeviceSignIn, Registration)>>,
}

#[derive(Clone)]
pub(crate) struct Context {
    pub accounts: Arc<Accounts>,
    /// The mail kept on this computer, which goes when its account does.
    pub mailboxes: Arc<crate::mailboxes::Mailboxes>,
    pub open_browser: OpenBrowser,
    pub default_client_id: Option<String>,
    pub device_sign_ins: Arc<DeviceSignIns>,
}

/// Which app registration to sign in under. Shared by both ways of signing in.
#[derive(Deserialize)]
struct Under {
    client_id: Option<String>,
    tenant: Option<String>,
}

#[derive(Deserialize)]
struct SignIn {
    #[serde(flatten)]
    under: Under,
    login_hint: Option<String>,
    /// An account already here, to sign in to again as it was first signed in.
    account_id: Option<String>,
}

#[derive(Deserialize)]
struct FlowId {
    flow_id: String,
}

#[derive(Deserialize)]
struct AccountId {
    id: String,
}

impl Context {
    fn registration(&self, under: Under) -> Result<Registration, CommandError> {
        let client_id = under
            .client_id
            .filter(|id| !id.trim().is_empty())
            .or_else(|| self.default_client_id.clone())
            // The one that worked last time, so it is typed once.
            .or_else(|| self.accounts.remembered_client_id())
            .ok_or_else(|| {
                failed(
                    "Brolga does not have its own app registration with Microsoft yet. Enter the client ID your \
                     organisation has registered for it, under Advanced.",
                )
            })?;

        Ok(Registration::new(client_id.trim(), under.tenant))
    }

    async fn sign_in(self, args: SignIn) -> Result<Value, CommandError> {
        // Renewing an account uses the registration it was added under, and
        // offers its own name on the sign-in page.
        let (registration, login_hint) = match &args.account_id {
            Some(id) => {
                let account = self
                    .accounts
                    .list()
                    .map_err(failed)?
                    .into_iter()
                    .find(|a| &a.id == id)
                    .ok_or_else(|| failed(format!("There is no account `{id}`.")))?;
                let registration = Registration {
                    client_id: account.client_id,
                    tenant: account.tenant,
                };
                (registration, Some(account.username))
            }
            None => (self.registration(args.under)?, args.login_hint),
        };
        let client = self.accounts.client();

        let sign_in = BrowserSignIn::begin(
            client,
            &registration,
            SIGN_IN_SCOPES,
            login_hint.as_deref(),
            None,
        )
        .await
        .map_err(failed)?;
        (self.open_browser)(&sign_in.url).map_err(|e| {
            failed(format!(
                "Your browser could not be opened ({e}). Sign in with a code instead."
            ))
        })?;
        let tokens = sign_in.finish(client, BROWSER_WAIT).await.map_err(failed)?;

        self.added(&registration, tokens).await
    }

    async fn begin_with_code(self, under: Under) -> Result<Value, CommandError> {
        let registration = self.registration(under)?;
        let sign_in = DeviceSignIn::begin(self.accounts.client(), &registration, SIGN_IN_SCOPES)
            .await
            .map_err(failed)?;

        let flow_id = (self.device_sign_ins.next.fetch_add(1, Ordering::SeqCst) + 1).to_string();
        let shown = json!({
            "flow_id": flow_id,
            "user_code": sign_in.user_code,
            "verification_uri": sign_in.verification_uri,
            "message": sign_in.message,
            "expires_in_seconds": sign_in.expires_in.as_secs(),
        });
        self.device_sign_ins
            .waiting
            .lock()
            .await
            .insert(flow_id, (sign_in, registration));

        Ok(shown)
    }

    async fn finish_with_code(self, args: FlowId) -> Result<Value, CommandError> {
        let (sign_in, registration) =
            self.device_sign_ins.waiting.lock().await.remove(&args.flow_id).ok_or_else(|| {
                failed("That sign-in is not waiting any more. It may have finished, or run out of time. Start again.")
            })?;
        let tokens = sign_in
            .finish(self.accounts.client())
            .await
            .map_err(failed)?;

        self.added(&registration, tokens).await
    }

    async fn added(
        &self,
        registration: &Registration,
        tokens: brolga_auth::Tokens,
    ) -> Result<Value, CommandError> {
        let account = self
            .accounts
            .add(registration, tokens)
            .await
            .map_err(failed)?;

        Ok(json!({ "account": shown(&account) }))
    }

    async fn list(self) -> Result<Value, CommandError> {
        let accounts: Vec<Value> = self
            .accounts
            .list()
            .map_err(failed)?
            .iter()
            .map(shown)
            .collect();

        Ok(json!({ "accounts": accounts }))
    }

    async fn options(self) -> Result<Value, CommandError> {
        let remembered = self.accounts.remembered_client_id();

        Ok(json!({
            "needs_client_id": self.default_client_id.is_none() && remembered.is_none(),
            "remembered_client_id": remembered,
        }))
    }

    async fn remove(self, args: AccountId) -> Result<Value, CommandError> {
        // The saved sign-in and the account first: if the secret store
        // will not let the sign-in go, nothing is removed and it can be
        // tried again.
        let removed = self.accounts.remove(&args.id).await.map_err(|e| {
            failed(format!(
                "The account was not removed, because its saved sign-in could not be deleted: {e}"
            ))
        })?;
        // Then its mail on this computer, and that of the mailboxes shared with it.
        if removed {
            self.mailboxes
                .forget_account(&args.id)
                .await
                .map_err(failed)?;
        }

        Ok(json!({ "removed": removed }))
    }
}

/// An account as a command gives it out. Nothing here is secret.
fn shown(account: &Account) -> Value {
    json!({
        "id": account.id,
        "username": account.username,
        "display_name": account.display_name,
        "tenant_id": account.tenant_id,
        "needs_sign_in": account.needs_sign_in,
        "added_at": account.added_at,
    })
}

fn failed(reason: impl std::fmt::Display) -> CommandError {
    CommandError::Failed(reason.to_string())
}

fn account_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "Brolga's id for the account. Pass it to other commands." },
            "username": { "type": "string", "description": "The sign-in name, usually an email address." },
            "display_name": { "type": ["string", "null"] },
            "tenant_id": { "type": "string", "description": "The organisation's id." },
            "needs_sign_in": {
                "type": "boolean",
                "description": "True when the saved sign-in no longer works and the person must sign in again."
            },
            "added_at": { "type": "string" }
        },
        "required": ["id", "username", "display_name", "tenant_id", "needs_sign_in", "added_at"],
        "additionalProperties": false
    })
}

fn one_account() -> Value {
    json!({
        "type": "object",
        "properties": { "account": account_schema() },
        "required": ["account"],
        "additionalProperties": false
    })
}

fn under_properties() -> Value {
    json!({
        "client_id": {
            "type": "string",
            "description": "The client ID of an app registration the organisation made for Brolga. Leave out to use Brolga's own."
        },
        "tenant": {
            "type": "string",
            "description": "The organisation's tenant ID or domain, where client_id is its own registration. Leave out for any organisation."
        }
    })
}

/// Wrap a typed handler: read the arguments into `A`, then run it.
fn handler<A, F, Fut>(context: &Context, run: F) -> brolga_commands::Handler
where
    A: serde::de::DeserializeOwned + Send + 'static,
    F: Fn(Context, A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value, CommandError>> + Send + 'static,
{
    let context = context.clone();

    Arc::new(move |args| {
        let parsed = serde_json::from_value::<A>(args);
        let running = parsed.map(|args| run(context.clone(), args));

        Box::pin(async move {
            match running {
                Ok(running) => running.await,
                // The schema lets through only what the type can read, so this is a bug in one or the other.
                Err(e) => Err(failed(format!("the arguments could not be read: {e}"))),
            }
        })
    })
}

pub(crate) fn register(registry: &mut Registry, context: Context) -> Result<(), RegistryError> {
    registry.register(
        CommandSpec {
            id: "accounts.list".to_owned(),
            title: "List accounts".to_owned(),
            description: "The Microsoft 365 accounts signed in to this client, with whether each still has a working sign-in.".to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: Some("list_accounts".to_owned()),
            args_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            result_schema: json!({
                "type": "object",
                "properties": { "accounts": { "type": "array", "items": account_schema() } },
                "required": ["accounts"],
                "additionalProperties": false
            }),
            examples: vec![json!({})],
        },
        handler(&context, |context, _: serde_json::Map<String, Value>| context.list()),
    )?;

    let mut sign_in_properties = under_properties();
    sign_in_properties["login_hint"] = json!({
        "type": "string",
        "description": "An email address to fill in on the sign-in page."
    });
    sign_in_properties["account_id"] = json!({
        "type": "string",
        "description": "An account already here that needs to sign in again. It is signed in as it was first added, and the other arguments are ignored."
    });
    registry.register(
        CommandSpec {
            id: "accounts.sign_in".to_owned(),
            title: "Add an account".to_owned(),
            description: "Sign in to a Microsoft 365 work or school account in the person's own browser, and add it to this client. \
                          Waits up to five minutes for them to finish. Signing in again to an account already here renews it."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": sign_in_properties, "additionalProperties": false }),
            result_schema: one_account(),
            examples: vec![json!({})],
        },
        handler(&context, Context::sign_in),
    )?;

    registry.register(
        CommandSpec {
            id: "accounts.sign_in_options".to_owned(),
            title: "Sign-in options".to_owned(),
            description: "What signing in needs on this build of the client: whether a client ID has to be given, because this build has no app registration of its own.".to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "needs_client_id": { "type": "boolean" },
                    "remembered_client_id": {
                        "type": ["string", "null"],
                        "description": "The client ID the last successful sign-in on this computer used, which is used again when none is given."
                    }
                },
                "required": ["needs_client_id"],
                "additionalProperties": false
            }),
            examples: vec![json!({})],
        },
        handler(&context, |context, _: serde_json::Map<String, Value>| context.options()),
    )?;

    registry.register(
        CommandSpec {
            id: "accounts.sign_in_with_code.begin".to_owned(),
            title: "Add an account with a code".to_owned(),
            description: "Start signing in with a code, for when no browser on this computer can be used. Gives a short code and \
                          the page to enter it on, from any device. Then run accounts.sign_in_with_code.finish with the flow_id."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": under_properties(), "additionalProperties": false }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "flow_id": { "type": "string" },
                    "user_code": { "type": "string", "description": "The code for the person to type in." },
                    "verification_uri": { "type": "string", "description": "The page to type it in at." },
                    "message": { "type": "string", "description": "Ready-made instructions to show the person." },
                    "expires_in_seconds": { "type": "integer" }
                },
                "required": ["flow_id", "user_code", "verification_uri", "message", "expires_in_seconds"],
                "additionalProperties": false
            }),
            examples: vec![json!({})],
        },
        handler(&context, Context::begin_with_code),
    )?;

    registry.register(
        CommandSpec {
            id: "accounts.sign_in_with_code.finish".to_owned(),
            title: "Finish adding an account with a code".to_owned(),
            description: "Wait for the person to enter the code from accounts.sign_in_with_code.begin, then add the account. \
                          Returns when they have, or fails when they refuse or the code runs out."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "flow_id": { "type": "string", "description": "From accounts.sign_in_with_code.begin." } },
                "required": ["flow_id"],
                "additionalProperties": false
            }),
            result_schema: one_account(),
            // The flow the example of `begin` started: examples run in id order.
            examples: vec![json!({ "flow_id": "1" })],
        },
        handler(&context, Context::finish_with_code),
    )?;

    registry.register(
        CommandSpec {
            id: "accounts.remove".to_owned(),
            title: "Remove an account".to_owned(),
            description: "Remove an account from this client: forget its saved sign-in, and delete this computer's copy of its mail and of the \
                          mailboxes shared with it. Nothing is deleted from the mailbox itself."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "The account's id, from accounts.list." } },
                "required": ["id"],
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": { "removed": { "type": "boolean", "description": "False if there was no such account." } },
                "required": ["removed"],
                "additionalProperties": false
            }),
            examples: vec![json!({ "id": "no-such-account" })],
        },
        handler(&context, Context::remove),
    )?;

    Ok(())
}
