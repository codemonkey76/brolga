//! Bringing the window forward, and a `mailto:` link opened in it.
//!
//! A `mailto:` link comes from a web page or another program, so it is
//! written by a stranger: it is read with limits, and all it can do is fill
//! in a message for the person to look over and send themselves.

use std::sync::{Arc, OnceLock};

use brolga_commands::{CommandError, CommandKind, CommandSpec, Registry, RegistryError};
use brolga_store::GlobalDb;
use serde_json::{Value, json};

/// Brings the window forward, and hands it a message to start where there
/// is one. Set by whatever has a window; without one, nothing is shown.
pub type ShowWindow = Arc<dyn Fn(Option<Value>) + Send + Sync>;

/// The most addresses taken for any one of To, Cc and Bcc.
const MOST_ADDRESSES: usize = 50;
/// The longest address there can be (RFC 5321, section 4.5.3.1).
const LONGEST_ADDRESS: usize = 320;
const LONGEST_SUBJECT: usize = 255;
const LONGEST_BODY: usize = 20_000;
/// The longest link read at all.
const LONGEST_LINK: usize = 64 * 1024;

/// `%41` to `A`, and anything that is not that left as it is.
fn decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (bytes[at], bytes.get(at + 1), bytes.get(at + 2)) {
            (b'%', Some(&high), Some(&low)) if hex(high).is_some() && hex(low).is_some() => {
                out.push((hex(high).unwrap_or(0) * 16 + hex(low).unwrap_or(0)) as u8);
                at += 3;
            }
            (other, _, _) => {
                out.push(other);
                at += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).into_owned()
}

fn addresses(list: &str, into: &mut Vec<Value>) {
    for each in list.split(',') {
        let address = decoded(each);
        let address = address.trim();
        let plain = address.contains('@')
            && address.chars().count() <= LONGEST_ADDRESS
            && !address
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || "<>\"".contains(c));
        if plain && into.len() < MOST_ADDRESSES && !into.iter().any(|a| a["address"] == address) {
            into.push(json!({ "address": address, "name": null }));
        }
    }
}

/// One line, of no more than `most` characters.
fn line(text: &str, most: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(most)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// What a `mailto:` link (RFC 6068) asks to be written: who to, the subject
/// and the words. Nothing else in it is acted on; a link cannot attach a
/// file or choose who a message is from. `None` if it is not such a link.
pub fn mailto(link: &str) -> Option<Value> {
    let link = link.trim();
    if link.len() > LONGEST_LINK || !link.get(..7)?.eq_ignore_ascii_case("mailto:") {
        return None;
    }
    let (to_list, fields) = link[7..].split_once('?').unwrap_or((&link[7..], ""));

    let (mut to, mut cc, mut bcc) = (Vec::new(), Vec::new(), Vec::new());
    let (mut subject, mut body) = (String::new(), String::new());
    addresses(to_list, &mut to);
    for field in fields.split('&') {
        let (name, value) = field.split_once('=').unwrap_or((field, ""));
        match decoded(name).to_ascii_lowercase().as_str() {
            "to" => addresses(value, &mut to),
            "cc" => addresses(value, &mut cc),
            "bcc" => addresses(value, &mut bcc),
            "subject" => subject = line(&decoded(value), LONGEST_SUBJECT),
            "body" => {
                body = decoded(value)
                    .replace("\r\n", "\n")
                    .chars()
                    .filter(|c| *c == '\n' || !c.is_control())
                    .take(LONGEST_BODY)
                    .collect();
            }
            _ => {}
        }
    }

    Some(json!({ "to": to, "cc": cc, "bcc": bcc, "subject": subject, "body": body }))
}

/// Where the choices about this computer are kept.
const DESKTOP: &str = "desktop";

/// How Brolga sits on this computer, as the person chose. Both are off
/// until they are turned on: a program that stays when it is closed, or
/// starts by itself, is one that was asked to.
fn desktop(db: &GlobalDb) -> Value {
    let kept: Value = db
        .setting(DESKTOP)
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);

    json!({
        "keep_running": kept["keep_running"].as_bool().unwrap_or(false),
        "start_at_login": kept["start_at_login"].as_bool().unwrap_or(false),
    })
}

pub(crate) fn register(
    registry: &mut Registry,
    window: Arc<OnceLock<ShowWindow>>,
    global: Arc<GlobalDb>,
) -> Result<(), RegistryError> {
    let choices = json!({
        "type": "object",
        "properties": {
            "keep_running": { "type": "boolean", "description": "Closing the window leaves Brolga running, fetching mail and giving reminders, with an icon in the system tray to bring it back or quit." },
            "start_at_login": { "type": "boolean", "description": "Brolga starts, with no window, when the person signs in to this computer." }
        },
        "required": ["keep_running", "start_at_login"],
        "additionalProperties": false
    });
    let db = global.clone();
    registry.register(
        CommandSpec {
            id: "app.desktop.get".to_owned(),
            title: "How Brolga sits on this computer".to_owned(),
            description: "Whether Brolga keeps running when its window is closed, and whether it starts when the person signs in to this computer. Both are off until turned on."
                .to_owned(),
            kind: CommandKind::Query,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            result_schema: choices.clone(),
            examples: vec![json!({})],
        },
        Arc::new(move |_| {
            let db = db.clone();
            Box::pin(async move { Ok(desktop(&db)) })
        }),
    )?;
    registry.register(
        CommandSpec {
            id: "app.desktop.set".to_owned(),
            title: "Change how Brolga sits on this computer".to_owned(),
            description: "Choose whether Brolga keeps running when its window is closed, and whether it starts when the person signs in to this computer. Only what is given changes. \
                          This records the choice; the app acts on it."
                .to_owned(),
            kind: CommandKind::Action,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": { "keep_running": { "type": "boolean" }, "start_at_login": { "type": "boolean" } },
                "additionalProperties": false
            }),
            result_schema: choices,
            examples: vec![json!({ "keep_running": true }), json!({ "keep_running": false })],
        },
        Arc::new(move |args| {
            let db = global.clone();
            Box::pin(async move {
                let mut now = desktop(&db);
                for each in ["keep_running", "start_at_login"] {
                    if let Some(chosen) = args[each].as_bool() {
                        now[each] = json!(chosen);
                    }
                }
                db.set_setting(DESKTOP, &now.to_string())
                    .map_err(|e| CommandError::Failed(e.to_string()))?;

                Ok(now)
            })
        }),
    )?;

    let people = json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": { "address": { "type": "string" }, "name": { "type": ["string", "null"] } },
            "required": ["address"]
        }
    });

    registry.register(
        CommandSpec {
            id: "app.window.show".to_owned(),
            title: "Show the window".to_owned(),
            description: "Bring Brolga's window to the front. Given a mailto: link, a new message is started in it with who it is to, \
                          the subject and the words the link names, for the person to look over: nothing is sent. \
                          `shown` is false when Brolga is running with no window."
                .to_owned(),
            kind: CommandKind::Navigation,
            consequential: false,
            mcp_tool: None,
            args_schema: json!({
                "type": "object",
                "properties": {
                    "mailto": { "type": "string", "maxLength": LONGEST_LINK, "description": "A mailto: link, as a web page or another program hands one over." }
                },
                "additionalProperties": false
            }),
            result_schema: json!({
                "type": "object",
                "properties": {
                    "shown": { "type": "boolean" },
                    "writing": {
                        "type": ["object", "null"],
                        "properties": {
                            "to": people, "cc": people, "bcc": people,
                            "subject": { "type": "string" },
                            "body": { "type": "string" }
                        }
                    }
                },
                "required": ["shown", "writing"],
                "additionalProperties": false
            }),
            examples: vec![
                json!({}),
                json!({ "mailto": "mailto:sam@example.test?subject=Thursday&body=See%20you%20then." }),
            ],
        },
        Arc::new(move |args| {
            let window = window.clone();
            Box::pin(async move {
                let writing = match args["mailto"].as_str() {
                    Some(link) => Some(mailto(link).ok_or_else(|| {
                        CommandError::Failed("That is not a mailto: link.".to_owned())
                    })?),
                    None => None,
                };
                let shown = match window.get() {
                    Some(show) => {
                        show(writing.clone());
                        true
                    }
                    None => false,
                };

                Ok(json!({ "shown": shown, "writing": writing }))
            })
        }),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_fills_in_who_the_subject_and_the_words() {
        let asked = mailto(
            "MAILTO:sam@example.test,%6Ao@example.test?cc=kim@example.test&BCC=lee@example.test\
             &subject=Budget%20review%0D%0ABcc:%20x@evil.test&body=Line%20one%0D%0ALine%20two&to=pat@example.test",
        )
        .unwrap();

        let list = |field: &str| -> Vec<String> {
            asked[field]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a["address"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(
            list("to"),
            ["sam@example.test", "jo@example.test", "pat@example.test"]
        );
        assert_eq!(list("cc"), ["kim@example.test"]);
        assert_eq!(list("bcc"), ["lee@example.test"]);
        // A line break in a subject cannot start a header of its own.
        assert_eq!(asked["subject"], "Budget reviewBcc: x@evil.test");
        assert_eq!(asked["body"], "Line one\nLine two");
    }

    #[test]
    fn a_link_can_ask_for_nothing_else_and_only_so_much() {
        let asked = mailto(
            "mailto:?attach=/etc/passwd&from=boss@example.test&to=not-an-address,a@b%20c,%3Cx@y%3E",
        )
        .unwrap();
        assert_eq!(
            asked,
            json!({ "to": [], "cc": [], "bcc": [], "subject": "", "body": "" })
        );

        let many: Vec<String> = (0..500).map(|n| format!("p{n}@example.test")).collect();
        let long = format!(
            "mailto:{}?subject={}&body={}",
            many.join(","),
            "s".repeat(1000),
            "b".repeat(50_000)
        );
        let asked = mailto(&long).unwrap();
        assert_eq!(asked["to"].as_array().unwrap().len(), MOST_ADDRESSES);
        assert_eq!(asked["subject"].as_str().unwrap().len(), LONGEST_SUBJECT);
        assert_eq!(asked["body"].as_str().unwrap().len(), LONGEST_BODY);

        for not in ["https://example.test/", "mail", "", "mailto"] {
            assert_eq!(mailto(not), None, "{not}");
        }
        // Bytes that are not text, and a percent sign with nothing after it.
        assert!(mailto("mailto:a@example.test?subject=%FF%FE%&body=%4").is_some());
    }
}
