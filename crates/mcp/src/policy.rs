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
    /// undone, such as sending. These are refused, whatever the access,
    /// until this is set.
    pub allow_consequential: bool,
    /// The most messages it may send in any hour.
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

/// The command that sends a message, which is what the hourly limit counts.
const SEND: &str = "mail.drafts.send";
/// The command the server itself writes its record with. Never an agent's.
pub(crate) const RECORD: &str = "app.agent_log.record";

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
    ) -> Result<(), String> {
        let id = spec.id.as_str();
        if id == RECORD {
            return Err("The record of what agents do is not an agent's to write.".to_owned());
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
        if spec.consequential && !self.allow_consequential {
            return Err(format!(
                "`{id}` reaches other people or cannot be undone, so it needs the user's say-so. \
                 The user can do it in the app, or allow it by starting the server with `--allow-consequential`."
            ));
        }
        // A path names a file on the user's computer. An agent that could
        // attach any file by its path could send any file away, and one that
        // could save to any path could write over anything.
        if args.get("path").is_some_and(|path| !path.is_null()) {
            return Err(format!(
                "`{id}` cannot be given a `path` by an agent. Give the file's contents instead, where the command takes them."
            ));
        }

        if id == SEND {
            let hour = Duration::from_secs(3600);
            session.sends.retain(|at| at.elapsed() < hour);
            if session.sends.len() as u32 >= self.max_sends_per_hour {
                return Err(format!(
                    "This agent has sent {} messages in the last hour, which is the most it is allowed. Try again later.",
                    session.sends.len()
                ));
            }
            session.sends.push(Instant::now());
        }

        Ok(())
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

    fn allowed(policy: &Policy, spec: &CommandSpec, args: Value) -> bool {
        policy.permit(spec, &args, &mut Session::default()).is_ok()
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

        assert!(!allowed(&full, &send, json!({})));
        assert!(allowed(&trusted, &send, json!({})));
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
    }
}
