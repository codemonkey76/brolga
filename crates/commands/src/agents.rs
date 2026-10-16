//! The commands an agent is treated differently for, by id. They are kept
//! beside the registry so that a test can hold each against the commands
//! there really are: an id nothing answers to would guard nothing.

/// The commands that always write to other people, which the hourly limit
/// counts along with sending.
pub const WRITES_TO_PEOPLE: &[&str] = &[
    "mail.drafts.send",
    "calendar.events.cancel",
    "calendar.events.respond",
    "calendar.sharing.set",
];
/// The commands that invite people when they name any, counted then.
pub const INVITES: &[&str] = &["calendar.events.create", "calendar.events.update"];

/// What only a person does, whatever an agent has been allowed: signing in
/// and out, opening a web address in the user's browser (which would carry
/// away whatever was put in the address), and choosing whose remote
/// pictures load.
pub const NEVER: &[&str] = &[
    "accounts.remove",
    "accounts.sign_in",
    "accounts.sign_in_with_code.begin",
    "accounts.sign_in_with_code.finish",
    "app.desktop.set",
    "app.open_link",
    "mail.trusted_senders.set",
];

/// The person's own side of a say-so: seeing what an agent is waiting on,
/// whole, and answering it. These run only when the window asks; the
/// registry refuses them to the command line, the control channel and the
/// MCP server alike, so a program that can reach the client from outside
/// cannot answer for the person or pass itself off as a window.
pub const WINDOW_ONLY: &[&str] = &[
    "app.confirmations.answer",
    "app.confirmations.get",
    "app.confirmations.list",
    // Which device carries out a mailbox's instructions is chosen by hand
    // (docs/spec-mail-instructions.md, "The processing device").
    "mail.instructions.processor.release",
    "mail.instructions.processor.take",
    // The key to the person's own Anthropic account.
    "mail.instructions.model.set_key",
    // Outside tool servers: which a computer reaches, and as whom.
    "mcp_servers.add",
    "mcp_servers.set_token",
    "mcp_servers.sign_in",
    // Calls to outside tools: approving one, or letting one go unasked.
    "mail.instructions.approvals.decide",
    "mail.instructions.tools.set_unattended",
];

/// Arguments only a person gives, by command: an agent that sets one is
/// refused. Loading a message's remote pictures tells the sender it was
/// opened, and is the person's to choose.
pub const NEVER_ARGUMENTS: &[(&str, &str)] = &[
    ("mail.messages.get", "allow_remote_content"),
    ("mail.messages.get", "allow_for_trusted_senders"),
    // Where a mailbox's mail is sent to be read by a model is the person's
    // to agree to (docs/spec-mail-instructions.md, "Cloud consent").
    ("mail.instructions.model.set", "cloud_consent"),
    ("mail.instructions.model.set", "ollama_address"),
];

/// Commands that need the person's say-so only when they are given a
/// certain argument. Changing the addresses a contact is written to at
/// would send later mail astray, so that asks, while changing a phone
/// number or a note does not.
pub const ASKS_WHEN_GIVEN: &[(&str, &str)] = &[("contacts.update", "emails")];
