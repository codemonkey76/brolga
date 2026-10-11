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
