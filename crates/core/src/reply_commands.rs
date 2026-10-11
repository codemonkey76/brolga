//! Automatic replies: the answer a mailbox gives while its owner is away.
//!
//! These are the mailbox's own settings, kept by Microsoft 365 and acted on
//! there, so they are read and set straight on the server and need it in reach.

use std::future::Future;
use std::sync::Arc;

use brolga_commands::{CommandError, CommandKind, CommandSpec, Handler, Registry, RegistryError};
use brolga_mail::{html_to_text, text_to_paragraphs};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mailboxes::Mailboxes;

#[derive(Deserialize)]
struct Get {
    account_id: Option<String>,
}

#[derive(Deserialize)]
struct Set {
    account_id: Option<String>,
    status: String,
    internal_message: Option<String>,
    external_message: Option<String>,
    external_audience: Option<String>,
    start: Option<String>,
    end: Option<String>,
}

/// The longest reply taken.
const LONGEST: usize = 8_000;

fn failed(reason: impl std::fmt::Display) -> CommandError {
    CommandError::Failed(reason.to_string())
}

/// Graph's setting as this client describes it: plain words, and plain
/// names for the choices.
fn shown(setting: &Value) -> Value {
    let text = |key: &str| html_to_text(setting[key].as_str().unwrap_or_default());
    let when = |key: &str| {
        let at = &setting[key];
        match at["dateTime"].as_str() {
            Some(date_time) => json!({
                // Graph writes seven places of seconds; nothing here needs them.
                "date_time": date_time.split('.').next().unwrap_or(date_time),
                "time_zone": at["timeZone"].as_str().unwrap_or("UTC"),
            }),
            None => Value::Null,
        }
    };

    json!({
        "status": match setting["status"].as_str().unwrap_or_default() {
            "alwaysEnabled" => "on",
            "scheduled" => "scheduled",
            _ => "off",
        },
        "internal_message": text("internalReplyMessage"),
        "external_message": text("externalReplyMessage"),
        "external_audience": match setting["externalAudience"].as_str().unwrap_or_default() {
            "all" => "everyone",
            "contactsOnly" => "contacts",
            _ => "nobody",
        },
        "start": when("scheduledStartDateTime"),
        "end": when("scheduledEndDateTime"),
    })
}

/// `2026-10-20T08:00:00Z`, taken apart far enough to be sure it is a time
/// in UTC and to compare two of them, and given back without the `Z`.
fn utc(time: &str) -> Option<&str> {
    let bare = time.strip_suffix('Z')?;
    let bytes = bare.as_bytes();
    let digits = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .is_some_and(|b| b.iter().all(u8::is_ascii_digit))
    };
    let shape = bytes.len() == 19
        && digits(0..4)
        && bytes[4] == b'-'
        && digits(5..7)
        && bytes[7] == b'-'
        && digits(8..10)
        && bytes[10] == b'T'
        && digits(11..13)
        && bytes[13] == b':'
        && digits(14..16)
        && bytes[16] == b':'
        && digits(17..19);

    shape.then_some(bare)
}

async fn get(mailboxes: Arc<Mailboxes>, args: Get) -> Result<Value, CommandError> {
    let account = mailboxes
        .account(args.account_id.as_deref())
        .map_err(failed)?;
    let mailbox = mailboxes.open(&account).await.map_err(failed)?;
    let setting = mailbox.graph.automatic_replies().await.map_err(explained)?;

    Ok(shown(&setting))
}

async fn set(mailboxes: Arc<Mailboxes>, args: Set) -> Result<Value, CommandError> {
    let mut setting = serde_json::Map::new();
    setting.insert(
        "status".to_owned(),
        json!(match args.status.as_str() {
            "on" => "alwaysEnabled",
            "scheduled" => "scheduled",
            _ => "disabled",
        }),
    );

    for (given, key) in [
        (&args.internal_message, "internalReplyMessage"),
        (&args.external_message, "externalReplyMessage"),
    ] {
        if let Some(text) = given {
            if text.chars().count() > LONGEST {
                return Err(failed(format!(
                    "An automatic reply can be at most {LONGEST} characters."
                )));
            }
            setting.insert(key.to_owned(), json!(text_to_paragraphs(text)));
        }
    }
    if let Some(audience) = &args.external_audience {
        setting.insert(
            "externalAudience".to_owned(),
            json!(match audience.as_str() {
                "everyone" => "all",
                "contacts" => "contactsOnly",
                _ => "none",
            }),
        );
    }

    if args.status == "scheduled" {
        let (Some(start), Some(end)) = (
            args.start.as_deref().and_then(utc),
            args.end.as_deref().and_then(utc),
        ) else {
            return Err(failed(
                "Scheduled replies need a start and an end, each a time in UTC such as 2026-10-20T08:00:00Z.",
            ));
        };
        // Written the same way, two such times compare as their text does.
        if end <= start {
            return Err(failed("The end must be after the start."));
        }
        setting.insert(
            "scheduledStartDateTime".to_owned(),
            json!({ "dateTime": start, "timeZone": "UTC" }),
        );
        setting.insert(
            "scheduledEndDateTime".to_owned(),
            json!({ "dateTime": end, "timeZone": "UTC" }),
        );
    }

    let account = mailboxes
        .account(args.account_id.as_deref())
        .map_err(failed)?;
    let mailbox = mailboxes.open(&account).await.map_err(failed)?;
    mailbox
        .graph
        .set_automatic_replies(&Value::Object(setting))
        .await
        .map_err(explained)?;
    let now = mailbox.graph.automatic_replies().await.map_err(explained)?;

    Ok(shown(&now))
}

/// A sign-in from before this was built was not asked for the permission.
fn explained(error: brolga_graph::GraphError) -> CommandError {
    match error {
        brolga_graph::GraphError::Refused { status: 403, .. } => failed(
            "This sign-in was not given permission for mailbox settings. Sign in to the account again, from the accounts screen, to grant it.",
        ),
        other => failed(other),
    }
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
    let when = json!({
        "anyOf": [
            { "type": "null" },
            {
                "type": "object",
                "properties": {
                    "date_time": { "type": "string", "description": "Without an offset, in `time_zone`." },
                    "time_zone": { "type": "string", "description": "UTC when set by this client. Set elsewhere, it may be a Windows time zone name." }
                },
                "required": ["date_time", "time_zone"]
            }
        ]
    });
    let result = json!({
        "type": "object",
        "properties": {
            "status": { "enum": ["off", "on", "scheduled"] },
            "internal_message": { "type": "string", "description": "What people in the same organisation are sent, as plain text." },
            "external_message": { "type": "string" },
            "external_audience": { "enum": ["nobody", "contacts", "everyone"], "description": "Who outside the organisation is sent the external message." },
            "start": when,
            "end": when
        },
        "required": ["status", "internal_message", "external_message", "external_audience", "start", "end"]
    });

    registry.register(
        CommandSpec {
            id: "mail.automatic_replies.get".to_owned(),
            title: "Read automatic replies".to_owned(),
            description: "The mailbox's automatic replies (out of office): whether they are off, on or scheduled, the messages, \
                          who outside the organisation gets one, and the scheduled times. Read from Microsoft 365, so it needs the network."
                .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: Some("get_automatic_replies".to_owned()),
            args_schema: json!({
                "type": "object",
                "properties": { "account_id": account_id },
                "additionalProperties": false
            }),
            result_schema: result.clone(),
            examples: vec![json!({})],
        },
        handler(&mailboxes, get),
    )?;

    registry.register(
        CommandSpec {
            id: "mail.automatic_replies.set".to_owned(),
            title: "Set automatic replies".to_owned(),
            description: "Turn the mailbox's automatic replies off or on, or schedule them between two times in UTC. \
                          Messages are plain text; one left out stays as it is. `external_audience` says who outside the organisation is answered. \
                          This takes effect at once on Microsoft 365, and everyone who writes while it is on is sent the reply."
                .to_owned(),
            kind: CommandKind::Action,
            // It writes to other people in the owner's name until turned off.
            consequential: true,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "account_id": account_id,
                    "status": { "enum": ["off", "on", "scheduled"] },
                    "internal_message": { "type": "string", "maxLength": LONGEST },
                    "external_message": { "type": "string", "maxLength": LONGEST },
                    "external_audience": { "enum": ["nobody", "contacts", "everyone"] },
                    "start": { "type": "string", "description": "For scheduled: UTC, as 2026-10-20T08:00:00Z." },
                    "end": { "type": "string" }
                },
                "required": ["status"],
                "additionalProperties": false
            }),
            result_schema: result,
            examples: vec![json!({ "status": "off" })],
        },
        handler(&mailboxes, set),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_must_be_whole_and_in_utc() {
        assert_eq!(utc("2026-10-20T08:00:00Z"), Some("2026-10-20T08:00:00"));
        for bad in [
            "2026-10-20T08:00:00",
            "2026-10-20T08:00:00+10:00",
            "2026-10-20 08:00:00Z",
            "tomorrow",
            "2026-10-20T08:00Z",
            "",
        ] {
            assert_eq!(utc(bad), None, "{bad}");
        }
    }

    #[test]
    fn graphs_setting_is_put_in_plain_words() {
        let setting = json!({
            "status": "scheduled",
            "externalAudience": "contactsOnly",
            "internalReplyMessage": "<html><body><p>Back on <b>Monday</b>.</p></body></html>",
            "externalReplyMessage": "",
            "scheduledStartDateTime": { "dateTime": "2026-10-20T08:00:00.0000000", "timeZone": "UTC" },
            "scheduledEndDateTime": { "dateTime": "2026-10-24T08:00:00.0000000", "timeZone": "AUS Eastern Standard Time" }
        });

        let shown = shown(&setting);

        assert_eq!(shown["status"], "scheduled");
        assert_eq!(shown["external_audience"], "contacts");
        assert_eq!(shown["internal_message"], "Back on Monday.");
        assert_eq!(
            shown["start"],
            json!({ "date_time": "2026-10-20T08:00:00", "time_zone": "UTC" })
        );
        assert_eq!(shown["end"]["time_zone"], "AUS Eastern Standard Time");
    }
}
