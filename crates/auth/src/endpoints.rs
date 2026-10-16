use serde::{Deserialize, Serialize};

/// What is asked for at sign-in: who the person is, a refresh token, their
/// own profile and their mail. A scope is added here when the feature that
/// needs it is built, so nobody is asked to consent to more than the client
/// can yet do.
pub const SIGN_IN_SCOPES: &[&str] = &[
    "openid",
    "profile",
    "offline_access",
    "User.Read",
    // Reading mail, and changing it: read state, flags, moves, drafts.
    "Mail.ReadWrite",
    "Mail.Send",
    // Automatic replies.
    "MailboxSettings.ReadWrite",
    // The calendar, to read and to change.
    "Calendars.ReadWrite",
    // Meeting times suggested by the server ("user: findMeetingTimes"), and
    // the calendars other people have shared or delegated.
    "Calendars.ReadWrite.Shared",
    // The organisation's meeting rooms ("List place objects") need
    // `Place.Read.All`, which only an administrator can give. It is not
    // asked for here, or nobody else could sign in at all: where an
    // administrator has approved it for the organisation, every token
    // carries it anyway, and rooms can be listed.
    // Personal contacts, and looking people up in the organisation.
    "Contacts.ReadWrite",
    "People.Read",
    "User.ReadBasic.All",
    // Mailboxes other people have shared: to read, change and send from.
    "Mail.ReadWrite.Shared",
    "Mail.Send.Shared",
    // Microsoft To Do: the person's task lists and tasks
    // (docs/spec-mail-instructions.md, "Tasks").
    "Tasks.ReadWrite",
];

/// The app registration a sign-in is made under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    /// The application (client) ID from the Entra admin centre.
    pub client_id: String,
    /// `organizations` for any work or school account, or one organisation's
    /// tenant ID or domain where it uses its own registration.
    pub tenant: String,
}

impl Registration {
    pub fn new(client_id: impl Into<String>, tenant: Option<String>) -> Self {
        Self {
            client_id: client_id.into(),
            tenant: tenant
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "organizations".to_owned()),
        }
    }
}

/// The identity platform's address. Only tests have reason to change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authority(String);

impl Default for Authority {
    fn default() -> Self {
        Self("https://login.microsoftonline.com".to_owned())
    }
}

impl Authority {
    pub fn new(base: impl Into<String>) -> Self {
        Self(base.into().trim_end_matches('/').to_owned())
    }

    pub(crate) fn authorize(&self, tenant: &str) -> String {
        format!("{}/{tenant}/oauth2/v2.0/authorize", self.0)
    }

    pub(crate) fn token(&self, tenant: &str) -> String {
        format!("{}/{tenant}/oauth2/v2.0/token", self.0)
    }

    pub(crate) fn device_code(&self, tenant: &str) -> String {
        format!("{}/{tenant}/oauth2/v2.0/devicecode", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registration_with_no_tenant_is_for_any_organisation() {
        assert_eq!(Registration::new("id", None).tenant, "organizations");
        assert_eq!(
            Registration::new("id", Some("  ".to_owned())).tenant,
            "organizations"
        );
        assert_eq!(
            Registration::new("id", Some("contoso.example".to_owned())).tenant,
            "contoso.example"
        );
    }

    #[test]
    fn the_endpoints_are_the_documented_v2_ones() {
        let authority = Authority::default();

        assert_eq!(
            authority.authorize("organizations"),
            "https://login.microsoftonline.com/organizations/oauth2/v2.0/authorize"
        );
        assert_eq!(
            authority.token("t"),
            "https://login.microsoftonline.com/t/oauth2/v2.0/token"
        );
        assert_eq!(
            authority.device_code("t"),
            "https://login.microsoftonline.com/t/oauth2/v2.0/devicecode"
        );
    }
}
