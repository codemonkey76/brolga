//! The commands for who may see one's own calendar.
//!
//! The list is kept by Microsoft 365 and nothing of it is kept here: it is
//! read and changed while the server can be reached. Only the main calendar
//! has one that Graph documents, and only its owner may read or change it.

use std::future::Future;
use std::sync::Arc;

use brolga_commands::{CommandError, CommandKind, CommandSpec, Handler, Registry, RegistryError};
use brolga_graph::GraphError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mailboxes::{Mailbox, Mailboxes};

/// What someone may be let do, least first ("calendarPermission resource type", SOURCES.md).
const ROLES: [&str; 6] = [
    "freeBusyRead",
    "limitedRead",
    "read",
    "write",
    "delegateWithoutPrivateEventAccess",
    "delegateWithPrivateEventAccess",
];

fn failed(reason: impl std::fmt::Display) -> CommandError {
    CommandError::Failed(reason.to_string())
}

/// The mailbox, which must be one's own: Graph gives a calendar's sharing
/// to its owner and to nobody else.
async fn own(
    mailboxes: &Mailboxes,
    account_id: Option<&str>,
) -> Result<Arc<Mailbox>, CommandError> {
    let account = mailboxes.account(account_id).map_err(failed)?;
    let mailbox = mailboxes.open(&account).await.map_err(failed)?;
    if mailbox.shared.is_some() {
        return Err(failed(
            "Only the owner of a calendar can see or change who it is shared with. Give the id of your own account.",
        ));
    }

    Ok(mailbox)
}

fn unreached(error: GraphError, then: &str) -> CommandError {
    if error.is_transient() {
        failed(format!(
            "Microsoft 365 could not be reached, so {then}. Sharing is kept there and can only be read or changed while it can be."
        ))
    } else {
        failed(error)
    }
}

fn shown(permission: &Value) -> Value {
    json!({
        "id": permission["id"],
        "name": permission["emailAddress"]["name"].as_str().unwrap_or_default(),
        // Null for the organisation as a whole.
        "address": permission["emailAddress"]["address"].as_str(),
        "role": permission["role"].as_str().unwrap_or("none"),
        "allowed_roles": permission["allowedRoles"].as_array().cloned().unwrap_or_default(),
        "removable": permission["isRemovable"].as_bool().unwrap_or(false),
        "inside_organization": permission["isInsideOrganization"].as_bool().unwrap_or(false),
    })
}

async fn listed(mailbox: &Mailbox, then: &str) -> Result<Vec<Value>, CommandError> {
    mailbox
        .graph
        .calendar_permissions()
        .await
        .map_err(|e| unreached(e, then))
}

#[derive(Deserialize)]
struct Account {
    account_id: Option<String>,
}

async fn list(mailboxes: Arc<Mailboxes>, args: Account) -> Result<Value, CommandError> {
    let mailbox = own(&mailboxes, args.account_id.as_deref()).await?;
    let all = listed(
        &mailbox,
        "who the calendar is shared with could not be read",
    )
    .await?;

    Ok(json!({ "shares": all.iter().map(shown).collect::<Vec<_>>() }))
}

#[derive(Deserialize)]
struct Set {
    account_id: Option<String>,
    address: Option<String>,
    share_id: Option<String>,
    name: Option<String>,
    role: String,
}

/// Share the calendar with someone, or change what someone it is already
/// shared with may do.
async fn set(mailboxes: Arc<Mailboxes>, args: Set) -> Result<Value, CommandError> {
    let mailbox = own(&mailboxes, args.account_id.as_deref()).await?;
    let all = listed(&mailbox, "the sharing was not changed").await?;
    let address = args.address.as_deref().map(|a| a.trim().to_lowercase());
    let existing = all.iter().find(|each| match (&args.share_id, &address) {
        (Some(id), _) => each["id"] == id.as_str(),
        (None, Some(address)) => each["emailAddress"]["address"]
            .as_str()
            .is_some_and(|theirs| theirs.eq_ignore_ascii_case(address)),
        (None, None) => false,
    });

    let changed = match (existing, address) {
        (Some(existing), _) => {
            let allowed = existing["allowedRoles"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !allowed.is_empty() && !allowed.iter().any(|each| each == args.role.as_str()) {
                let allowed: Vec<&str> = allowed.iter().filter_map(Value::as_str).collect();
                return Err(failed(format!(
                    "Microsoft 365 will not let them be given `{}`. What they may be given is: {}.",
                    args.role,
                    allowed.join(", ")
                )));
            }
            mailbox
                .graph
                .update_calendar_permission(existing["id"].as_str().unwrap_or_default(), &args.role)
                .await
                .map_err(|e| unreached(e, "the sharing may not have been changed"))?
        }
        (None, Some(address)) if args.share_id.is_none() => {
            if !address.contains('@') || address.contains(char::is_whitespace) {
                return Err(failed(format!("`{address}` is not an email address.")));
            }
            if args.role == "none" {
                return Err(failed(
                    "They do not have the calendar, so there is nothing to take away.",
                ));
            }
            // Whether they are of the same organisation is the server's to
            // say; it is asked for what the address suggests.
            let domain = |address: &str| address.rsplit('@').next().map(str::to_lowercase);
            let own_address = mailboxes
                .account(args.account_id.as_deref())
                .map_err(failed)?
                .username;
            let permission = json!({
                "emailAddress": { "address": address, "name": args.name.as_deref().unwrap_or(&address) },
                "isInsideOrganization": domain(&address) == domain(&own_address),
                "isRemovable": true,
                "role": args.role,
            });
            match mailbox.graph.share_calendar(&permission).await {
                Ok(made) => made,
                Err(error) if error.is_transient() => {
                    // It may have been done: look, and do not ask twice.
                    listed(&mailbox, "the calendar may or may not have been shared; look before trying again")
                        .await?
                        .into_iter()
                        .find(|each| {
                            each["emailAddress"]["address"]
                                .as_str()
                                .is_some_and(|theirs| theirs.eq_ignore_ascii_case(&address))
                        })
                        .ok_or_else(|| {
                            failed("Microsoft 365 could not be reached, so the calendar was not shared. Try again when it can be.")
                        })?
                }
                Err(error) => return Err(failed(error)),
            }
        }
        (None, _) if args.share_id.is_some() => {
            return Err(failed(
                "There is no share with that id. Those there are, are in calendar.sharing.list.",
            ));
        }
        (None, _) => return Err(failed("Say who: give their address, or the id of a share.")),
    };

    Ok(json!({ "share": shown(&changed) }))
}

#[derive(Deserialize)]
struct Remove {
    account_id: Option<String>,
    share_id: String,
}

async fn remove(mailboxes: Arc<Mailboxes>, args: Remove) -> Result<Value, CommandError> {
    let mailbox = own(&mailboxes, args.account_id.as_deref()).await?;
    let all = listed(&mailbox, "the sharing was not changed").await?;
    let Some(share) = all.iter().find(|each| each["id"] == args.share_id.as_str()) else {
        // Already not shared with them.
        return Ok(json!({ "removed": false }));
    };
    if share["isRemovable"] == false {
        return Err(failed(
            "That one cannot be removed. What your organisation as a whole may see can be changed, down to `none`, with calendar.sharing.set.",
        ));
    }
    mailbox
        .graph
        .delete_calendar_permission(&args.share_id)
        .await
        .map_err(|e| unreached(e, "the sharing may not have been changed"))?;

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
    let account_id = json!({ "type": "string", "description": "Which account, from accounts.list. May be left out when there is only one." });
    let share = json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "name": { "type": "string" },
            "address": { "type": ["string", "null"], "description": "Null for the organisation as a whole." },
            "role": { "type": "string", "description": "none, freeBusyRead (when you are busy), limitedRead (and titles and places), read (everything but private events), write, delegateWithoutPrivateEventAccess, delegateWithPrivateEventAccess, or custom." },
            "allowed_roles": { "type": "array", "items": { "type": "string" }, "description": "What Microsoft 365 will let them be given." },
            "removable": { "type": "boolean" },
            "inside_organization": { "type": "boolean" }
        },
        "required": ["id", "name", "address", "role", "allowed_roles", "removable"]
    });
    const WHERE: &str = "This is your main calendar's sharing, kept by Microsoft 365, so it works only while that can be reached.";

    registry.register(
        CommandSpec {
            id: "calendar.sharing.list".to_owned(),
            title: "Who the calendar is shared with".to_owned(),
            description: format!("Everyone your main calendar is shared with or delegated to, and what each may do, with your organisation as a whole among them. {WHERE}"),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": { "account_id": account_id }, "additionalProperties": false }),
            result_schema: json!({ "type": "object", "properties": { "shares": { "type": "array", "items": share } }, "required": ["shares"] }),
            examples: vec![json!({})],
        },
        handler(&mailboxes, list),
    )?;

    registry.register(
        CommandSpec {
            id: "calendar.sharing.remove".to_owned(),
            title: "Stop sharing the calendar".to_owned(),
            description: format!("Stop sharing your main calendar with one person, by the id of their share. They can no longer see it. {WHERE}"),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id, "share_id": { "type": "string", "description": "From calendar.sharing.list." } },
                "required": ["share_id"],
                "additionalProperties": false
            }),
            result_schema: json!({ "type": "object", "properties": { "removed": { "type": "boolean" } }, "required": ["removed"] }),
            examples: vec![json!({ "share_id": "no-such-share" })],
        },
        handler(&mailboxes, remove),
    )?;

    registry.register(
        CommandSpec {
            id: "calendar.sharing.set".to_owned(),
            title: "Share the calendar".to_owned(),
            description: format!(
                "Share your main calendar with someone, by their address, or change what someone it is already shared with may do, by their address or the id of their share. \
                 The role says how much: freeBusyRead shows only when you are busy, limitedRead adds titles and places, read shows everything but private events, \
                 write lets them change it, and the two delegate roles let them act for you. Microsoft 365 allows people outside your organisation less, \
                 and says what in allowed_roles. From then on they see your calendar, so be sure of the address. {WHERE}"
            ),
            kind: CommandKind::Action,
            // It shows the calendar to someone else, and may let them act in the owner's name.
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id,
                    "address": { "type": "string", "description": "Who to share with, or whose share to change." },
                    "share_id": { "type": "string", "description": "A share from calendar.sharing.list, in place of an address: the only way to name the organisation as a whole." },
                    "name": { "type": "string", "maxLength": 255, "description": "Their name, for a new share." },
                    "role": { "enum": ["none", ROLES[0], ROLES[1], ROLES[2], ROLES[3], ROLES[4], ROLES[5]] }
                },
                "required": ["role"],
                "additionalProperties": false
            }),
            result_schema: json!({ "type": "object", "properties": { "share": share }, "required": ["share"] }),
            examples: vec![json!({ "address": "sam@example.test", "name": "Sam Taylor", "role": "read" })],
        },
        handler(&mailboxes, set),
    )?;

    Ok(())
}
