//! The commands for mailboxes other people have shared.
//!
//! Graph will not say which mailboxes someone has been given, so one is
//! added by its address. Once added it is a mailbox like any other: it has
//! an id, which goes wherever an account id is taken, and it syncs with the
//! account it was added to.

use std::future::Future;
use std::sync::Arc;

use brolga_commands::{CommandError, CommandKind, CommandSpec, Handler, Registry, RegistryError};
use brolga_graph::GraphError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mailboxes::{Mailboxes, SharedMailbox};

fn failed(reason: impl std::fmt::Display) -> CommandError {
    CommandError::Failed(reason.to_string())
}

fn shown(account_id: &str, shared: &SharedMailbox) -> Value {
    json!({
        "id": shared.id(account_id), "account_id": account_id, "address": shared.address,
        "name": shared.name, "calendar_only": shared.calendar_only
    })
}

#[derive(Deserialize)]
struct Add {
    account_id: Option<String>,
    address: String,
}

async fn add(mailboxes: Arc<Mailboxes>, args: Add) -> Result<Value, CommandError> {
    let account = mailboxes
        .account(args.account_id.as_deref())
        .map_err(failed)?;
    if account.id.contains('~') {
        return Err(failed(
            "A shared mailbox is added to an account, not to another shared mailbox.",
        ));
    }
    let address = args.address.trim().to_lowercase();
    if !address.contains('@')
        || address.contains(|c: char| c.is_whitespace() || c == '~' || c == '/')
    {
        return Err(failed(format!("`{address}` is not an email address.")));
    }
    if address.eq_ignore_ascii_case(&account.username) {
        return Err(failed("That is this account's own mailbox."));
    }
    let mut all = mailboxes.shared(&account.id).map_err(failed)?;
    if all.iter().any(|each| each.address == address) {
        return Err(failed(format!(
            "The mailbox `{address}` has already been added."
        )));
    }

    // Its name, if the directory will say; the address stands in otherwise.
    let own = mailboxes.open(&account).await.map_err(failed)?;
    let name = own
        .graph
        .users_search(&address)
        .await
        .ok()
        .and_then(|found| {
            found
                .iter()
                .find(|each| {
                    each["mail"]
                        .as_str()
                        .is_some_and(|mail| mail.eq_ignore_ascii_case(&address))
                })
                .and_then(|each| each["displayName"].as_str().map(str::to_owned))
        })
        .unwrap_or_default();
    let mut shared = SharedMailbox {
        address: address.clone(),
        name,
        calendar_only: false,
    };

    // It is added, and then looked into: if the inbox cannot be reached it
    // is taken off again, and the server's own words say why.
    all.push(shared.clone());
    mailboxes.set_shared(&account.id, &all).map_err(failed)?;
    let id = shared.id(&account.id);
    let reached = async {
        let standing = mailboxes.account(Some(&id)).map_err(failed)?;
        let mailbox = mailboxes.open(&standing).await.map_err(failed)?;
        let mail = mailbox.graph.well_known_folder("inbox").await;
        // With no way into the mail, the calendar alone may have been shared.
        if matches!(
            mail,
            Ok(None) | Err(GraphError::Refused { status: 403 | 404, .. })
        ) && mailbox.graph.calendars().await.is_ok()
        {
            return Ok(true);
        }
        match mail {
            Ok(Some(_)) => Ok(false),
            Ok(None) | Err(GraphError::Refused { status: 404, .. }) => Err(failed(format!(
                "There is no mailbox `{address}`, or you have not been given access to it or to its calendar."
            ))),
            Err(GraphError::Refused { status: 403, message, .. }) => Err(failed(format!(
                "You have not been given access to the mailbox `{address}` or to its calendar, or this sign-in is from before \
                 shared mailboxes could be opened: sign in to the account again to grant that. Microsoft 365 said: {message}"
            ))),
            Err(error) if error.is_transient() => Err(failed(
                "Microsoft 365 could not be reached, so the mailbox could not be looked into. Try again when it can.",
            )),
            Err(error) => Err(failed(error)),
        }
    }
    .await;
    match reached {
        Err(error) => {
            all.retain(|each| each.address != address);
            mailboxes.set_shared(&account.id, &all).map_err(failed)?;
            mailboxes.close(&id).await;
            if let Some(file) = mailboxes.file(&id) {
                let _ = std::fs::remove_file(file);
            }
            return Err(error);
        }
        Ok(true) => {
            // It is kept as a calendar, and opened afresh as one.
            shared.calendar_only = true;
            if let Some(kept) = all.iter_mut().find(|each| each.address == address) {
                kept.calendar_only = true;
            }
            mailboxes.set_shared(&account.id, &all).map_err(failed)?;
            mailboxes.close(&id).await;
        }
        Ok(false) => {}
    }

    Ok(json!({ "mailbox": shown(&account.id, &shared) }))
}

async fn list(
    mailboxes: Arc<Mailboxes>,
    _: serde_json::Map<String, Value>,
) -> Result<Value, CommandError> {
    let mut all = Vec::new();
    for account in mailboxes.accounts().map_err(failed)? {
        // The accounts themselves, and not what stands as one.
        if account.id.contains('~') {
            continue;
        }
        for shared in mailboxes.shared(&account.id).map_err(failed)? {
            all.push(shown(&account.id, &shared));
        }
    }

    Ok(json!({ "mailboxes": all }))
}

#[derive(Deserialize)]
struct Remove {
    mailbox_id: String,
}

async fn remove(mailboxes: Arc<Mailboxes>, args: Remove) -> Result<Value, CommandError> {
    let Some((owner, address)) = args.mailbox_id.split_once('~') else {
        return Err(failed(
            "That is not the id of a shared mailbox. See accounts.shared.list.",
        ));
    };
    let mut all = mailboxes.shared(owner).map_err(failed)?;
    let before = all.len();
    all.retain(|each| !each.address.eq_ignore_ascii_case(address));
    if all.len() == before {
        return Ok(json!({ "removed": false }));
    }
    mailboxes.set_shared(owner, &all).map_err(failed)?;
    // Its copy on this computer goes too. The mailbox itself is untouched.
    mailboxes.close(&args.mailbox_id).await;
    if let Some(file) = mailboxes.file(&args.mailbox_id) {
        for suffix in ["", "-wal", "-shm"] {
            let mut path = file.clone().into_os_string();
            path.push(suffix);
            let _ = std::fs::remove_file(path);
        }
    }

    Ok(json!({ "removed": true }))
}

fn handler<A, F, Fut>(mailboxes: &Arc<Mailboxes>, run: F) -> Handler
where
    A: serde::de::DeserializeOwned + Send + 'static,
    F: Fn(Arc<Mailboxes>, A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value, CommandError>> + Send + 'static,
{
    let mailboxes = mailboxes.clone();

    Arc::new(move |args| {
        let running = serde_json::from_value::<A>(args).map(|args| run(mailboxes.clone(), args));

        Box::pin(async move {
            match running {
                Ok(running) => running.await,
                Err(error) => Err(failed(error)),
            }
        })
    })
}

pub(crate) fn register(
    registry: &mut Registry,
    mailboxes: Arc<Mailboxes>,
) -> Result<(), RegistryError> {
    let mailbox = json!({
        "type": "object",
        "properties": {
            "id": { "type": "string", "description": "Give this wherever a command takes account_id, to work in this mailbox." },
            "account_id": { "type": "string", "description": "The account it was added to, whose sign-in opens it." },
            "address": { "type": "string" },
            "name": { "type": "string" },
            "calendar_only": { "type": "boolean", "description": "Only its calendar was shared: its id is for the calendar commands, and there is no mail of it to read." }
        },
        "required": ["id", "account_id", "address"]
    });

    registry.register(
        CommandSpec {
            id: "accounts.shared.add".to_owned(),
            title: "Add a shared mailbox".to_owned(),
            description: "Add a mailbox someone has given you access to, by its address: Microsoft 365 will not list them, so each is added \
                          by hand. It then syncs with the account, and its id goes wherever a command takes account_id. Where someone has shared \
                          only their calendar, that is added the same way and marked calendar_only. Nothing is changed in the mailbox."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": { "type": "string", "description": "The account that has access. May be left out when there is only one." },
                    "address": { "type": "string", "description": "The shared mailbox's email address." }
                },
                "required": ["address"],
                "additionalProperties": false
            }),
            result_schema: json!({ "type": "object", "properties": { "mailbox": mailbox }, "required": ["mailbox"] }),
            examples: vec![json!({ "address": "support@example.test" })],
        },
        handler(&mailboxes, add),
    )?;

    registry.register(
        CommandSpec {
            id: "accounts.shared.list".to_owned(),
            title: "Shared mailboxes".to_owned(),
            description: "The shared mailboxes that have been added, across every account, each with the id to use it by.".to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            result_schema: json!({
                "type": "object",
                "properties": { "mailboxes": { "type": "array", "items": mailbox } },
                "required": ["mailboxes"]
            }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, list),
    )?;

    registry.register(
        CommandSpec {
            id: "accounts.shared.remove".to_owned(),
            title: "Remove a shared mailbox".to_owned(),
            description: "Stop showing a shared mailbox and delete this computer's copy of it. The mailbox itself, and your access to it, are untouched."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "mailbox_id": { "type": "string", "description": "From accounts.shared.list." } },
                "required": ["mailbox_id"],
                "additionalProperties": false
            }),
            result_schema: json!({ "type": "object", "properties": { "removed": { "type": "boolean" } }, "required": ["removed"] }),
            examples: vec![json!({ "mailbox_id": "no-account~nobody@example.test" })],
        },
        handler(&mailboxes, remove),
    )?;

    Ok(())
}
