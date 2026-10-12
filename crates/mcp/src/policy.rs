//! What an agent is allowed to do.
//!
//! An agent starts able to read and nothing more. More is given by the
//! person who sets the agent up, never by the agent and never by anything it
//! reads: the text of an email cannot raise what an agent may do, because
//! what it may do is fixed before the first message is read.

use std::time::{Duration, Instant};

use brolga_commands::{Access, CommandSpec};
use serde_json::Value;

/// What an agent may do, as the person who set it up decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// How far the agent is trusted. An agent is given `Access::Read` unless
    /// told otherwise.
    pub access: Access,
    /// Whether it may run commands that reach other people or cannot be
    /// undone, such as sending, without the person being asked each time.
    /// Until this is set, each one is put to the person in the window and
    /// runs only on a yes.
    pub allow_consequential: bool,
    /// The most messages and invitations it may send in any hour.
    pub max_sends_per_hour: u32,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            access: Access::Read,
            allow_consequential: false,
            max_sends_per_hour: 20,
        }
    }
}

/// The command that sends a message.
#[cfg(test)]
const SEND: &str = "mail.drafts.send";
use brolga_commands::agents::{ASKS_WHEN_GIVEN, INVITES, NEVER, NEVER_ARGUMENTS, WRITES_TO_PEOPLE};

/// Whether this command, asked this way, sends something to other people.
fn reaches_people(id: &str, args: &Value) -> bool {
    WRITES_TO_PEOPLE.contains(&id)
        || (INVITES.contains(&id)
            && args
                .get("attendees")
                .and_then(Value::as_array)
                .is_some_and(|invited| !invited.is_empty()))
}
/// The command the server itself writes its record with. Never an agent's.
pub(crate) const RECORD: &str = "app.agent_log.record";

/// Whether an argument was given at all. A null is as good as leaving it
/// out, and a `false` asks for nothing.
fn given(args: &Value, name: &str) -> bool {
    args.get(name)
        .is_some_and(|value| !value.is_null() && value != &Value::Bool(false))
}

/// The command the server asks the user's say-so with, and the ones a
/// person sees the questions and answers them with. Never an agent's.
pub(crate) const ASK: &str = "app.confirmations.ask";
const CONFIRMATIONS: &str = "app.confirmations.";

/// What a permitted command still needs before it runs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Needs {
    Nothing,
    /// The say-so of the person at the window, asked for now.
    SaySo,
}

/// One agent's use of the server, for as long as it is connected.
#[derive(Default)]
pub(crate) struct Session {
    sends: Vec<Instant>,
}

impl Policy {
    /// Whether this agent may run this command with these arguments, and if
    /// not, why not, in words for the agent to pass on.
    pub(crate) fn permit(
        &self,
        spec: &CommandSpec,
        args: &Value,
        session: &mut Session,
    ) -> Result<Needs, String> {
        let id = spec.id.as_str();
        if id == RECORD {
            return Err("The record of what agents do is not an agent's to write.".to_owned());
        }
        if id.starts_with(CONFIRMATIONS) {
            return Err(
                "Asking for the user's say-so, seeing what is being asked, and giving it, are not an agent's to do."
                    .to_owned(),
            );
        }

        if NEVER.contains(&id) {
            return Err(format!(
                "`{id}` is the user's own to do, in the app. No agent may do it."
            ));
        }

        let needed = spec.access();
        if needed > self.access {
            return Err(match needed {
                Access::Draft => format!(
                    "`{id}` writes a draft, and this agent may only read. The user can allow drafting by starting the server with `--access draft`."
                ),
                _ => format!(
                    "`{id}` changes the mailbox, and this agent may not. The user can do it in the app, or allow it by starting the server with `--access full`."
                ),
            });
        }

        // A path names a file on the user's computer. An agent that could
        // attach any file by its path could send any file away, and one that
        // could save to any path could write over anything.
        if args.get("path").is_some_and(|path| !path.is_null()) {
            return Err(format!(
                "`{id}` cannot be given a `path` by an agent. Give the file's contents instead, where the command takes them."
            ));
        }

        // Whether a message's remote pictures are fetched is the person's
        // to choose: fetching them tells the sender the message was opened.
        if let Some((_, argument)) = NEVER_ARGUMENTS
            .iter()
            .find(|(command, argument)| *command == id && given(args, argument))
        {
            return Err(format!(
                "`{id}` cannot be given `{argument}` by an agent. Whether a message's remote pictures are fetched is the user's to choose, in the app."
            ));
        }

        if reaches_people(id, args) {
            let hour = Duration::from_secs(3600);
            session.sends.retain(|at| at.elapsed() < hour);
            if session.sends.len() as u32 >= self.max_sends_per_hour {
                return Err(format!(
                    "This agent has sent {} messages and invitations in the last hour, which is the most it is allowed. Try again later.",
                    session.sends.len()
                ));
            }
            session.sends.push(Instant::now());
        }

        // Last, so that the user is only ever asked about something that
        // would otherwise go ahead.
        let asks = spec.consequential
            || ASKS_WHEN_GIVEN
                .iter()
                .any(|(command, argument)| *command == id && args.get(argument).is_some());
        if asks && !self.allow_consequential {
            return Ok(Needs::SaySo);
        }

        Ok(Needs::Nothing)
    }

    /// What to tell an agent whose command needed a say-so it did not get.
    pub(crate) fn needs_say_so(id: &str, why: &str) -> String {
        format!(
            "`{id}` reaches other people or cannot be undone, so it needs the user's say-so. {why} \
             The user can do it in the app, or allow it by starting the server with `--allow-consequential`."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brolga_commands::CommandKind;
    use serde_json::json;

    fn spec(id: &str, kind: CommandKind, consequential: bool) -> CommandSpec {
        CommandSpec {
            id: id.to_owned(),
            title: String::new(),
            description: String::new(),
            kind,
            consequential,
            mcp_tool: None,
            args_schema: json!({}),
            result_schema: json!({}),
            examples: vec![],
        }
    }

    /// Goes ahead with nothing more asked.
    fn allowed(policy: &Policy, spec: &CommandSpec, args: Value) -> bool {
        policy.permit(spec, &args, &mut Session::default()) == Ok(Needs::Nothing)
    }

    #[test]
    fn an_agent_does_only_what_its_access_covers() {
        let read = spec("mail.messages.list", CommandKind::Query, false);
        let draft = spec("mail.drafts.create", CommandKind::Action, false);
        let change = spec("mail.messages.move", CommandKind::Action, false);
        let with = |access| Policy {
            access,
            ..Policy::default()
        };

        for (access, may) in [
            (Access::Read, [true, false, false]),
            (Access::Draft, [true, true, false]),
            (Access::Full, [true, true, true]),
        ] {
            let policy = with(access);
            assert_eq!(
                [&read, &draft, &change].map(|s| allowed(&policy, s, json!({}))),
                may,
                "{access:?}"
            );
        }
        // Nothing is given that was not asked for.
        assert_eq!(Policy::default().access, Access::Read);
    }

    #[test]
    fn sending_needs_its_own_say_so_even_with_full_access() {
        let send = spec(SEND, CommandKind::Action, true);
        let full = Policy {
            access: Access::Full,
            ..Policy::default()
        };
        let trusted = Policy {
            allow_consequential: true,
            ..full.clone()
        };

        // Without the flag it is not refused outright: the user is asked.
        assert_eq!(
            full.permit(&send, &json!({}), &mut Session::default()),
            Ok(Needs::SaySo)
        );
        assert!(allowed(&trusted, &send, json!({})));
        // An agent that may not change the mailbox is not worth asking about.
        let reader = Policy::default();
        assert!(
            reader
                .permit(&send, &json!({}), &mut Session::default())
                .is_err()
        );
    }

    #[test]
    fn an_agent_cannot_name_a_file_on_the_computer() {
        let attach = spec("mail.drafts.files.add", CommandKind::Action, false);
        let policy = Policy {
            access: Access::Full,
            allow_consequential: true,
            ..Policy::default()
        };

        assert!(!allowed(
            &policy,
            &attach,
            json!({ "draft_id": "draft-1", "path": "/home/robin/.ssh/id_ed25519" })
        ));
        assert!(allowed(
            &policy,
            &attach,
            json!({ "draft_id": "draft-1", "name": "a.txt", "content_base64": "SGk=" })
        ));
    }

    #[test]
    fn sends_are_limited_by_the_hour_and_the_record_is_never_an_agents() {
        let send = spec(SEND, CommandKind::Action, true);
        let policy = Policy {
            access: Access::Full,
            allow_consequential: true,
            max_sends_per_hour: 2,
        };
        let mut session = Session::default();

        assert!(policy.permit(&send, &json!({}), &mut session).is_ok());
        assert!(policy.permit(&send, &json!({}), &mut session).is_ok());
        let third = policy.permit(&send, &json!({}), &mut session);
        assert!(third.unwrap_err().contains("last hour"));

        let record = spec(RECORD, CommandKind::Action, false);
        assert!(!allowed(&policy, &record, json!({})));
        for (own, kind) in [
            (ASK, CommandKind::Action),
            ("app.confirmations.answer", CommandKind::Action),
            ("app.confirmations.list", CommandKind::Query),
            ("app.confirmations.get", CommandKind::Query),
        ] {
            assert!(
                policy
                    .permit(&spec(own, kind, false), &json!({}), &mut Session::default())
                    .is_err(),
                "{own}"
            );
        }
    }

    #[test]
    fn an_agent_cannot_have_a_messages_remote_pictures_fetched() {
        let get = spec("mail.messages.get", CommandKind::Query, false);
        let policy = Policy {
            access: Access::Full,
            allow_consequential: true,
            ..Policy::default()
        };

        for argument in ["allow_remote_content", "allow_for_trusted_senders"] {
            let refused = policy.permit(
                &get,
                &json!({ "message_id": "m-1", argument: true }),
                &mut Session::default(),
            );
            assert!(refused.unwrap_err().contains(argument), "{argument}");
        }
        // Reading is still a reader's, and saying no to pictures asks for nothing.
        assert!(allowed(
            &Policy::default(),
            &get,
            json!({ "message_id": "m-1", "format": "html", "allow_remote_content": false })
        ));
    }

    #[test]
    fn changing_where_a_contact_is_written_to_needs_a_say_so() {
        let update = spec("contacts.update", CommandKind::Action, false);
        let full = Policy {
            access: Access::Full,
            ..Policy::default()
        };

        assert_eq!(
            full.permit(
                &update,
                &json!({ "contact_id": "c-1", "emails": ["x@elsewhere.test"] }),
                &mut Session::default()
            ),
            Ok(Needs::SaySo)
        );
        // Taking every address away is a change to them too.
        assert_eq!(
            full.permit(
                &update,
                &json!({ "contact_id": "c-1", "emails": [] }),
                &mut Session::default()
            ),
            Ok(Needs::SaySo)
        );
        assert!(allowed(
            &full,
            &update,
            json!({ "contact_id": "c-1", "job_title": "Director" })
        ));
    }

    #[test]
    fn what_is_the_users_own_stays_so_whatever_the_agent_is_allowed() {
        let policy = Policy {
            access: Access::Full,
            allow_consequential: true,
            ..Policy::default()
        };

        for own in NEVER {
            let refused = policy.permit(
                &spec(own, CommandKind::Action, false),
                &json!({}),
                &mut Session::default(),
            );
            assert!(refused.unwrap_err().contains("No agent"), "{own}");
        }
    }

    #[test]
    fn invitations_count_towards_the_hourly_limit_and_plain_events_do_not() {
        let create = spec("calendar.events.create", CommandKind::Action, true);
        let cancel = spec("calendar.events.cancel", CommandKind::Action, true);
        let policy = Policy {
            access: Access::Full,
            allow_consequential: true,
            max_sends_per_hour: 2,
        };
        let mut session = Session::default();
        let alone = json!({ "subject": "Dentist" });
        let invited =
            json!({ "subject": "Review", "attendees": [{ "address": "kim@example.test" }] });

        for _ in 0..5 {
            assert!(policy.permit(&create, &alone, &mut session).is_ok());
        }
        assert!(policy.permit(&create, &invited, &mut session).is_ok());
        assert!(policy.permit(&cancel, &json!({}), &mut session).is_ok());
        assert!(
            policy
                .permit(&create, &invited, &mut session)
                .unwrap_err()
                .contains("last hour")
        );
        // Still free to do what reaches nobody.
        assert!(policy.permit(&create, &alone, &mut session).is_ok());
    }
}
