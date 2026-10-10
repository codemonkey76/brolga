/// Why a sign-in did not work, as far as the person signing in is concerned.
///
/// An organisation's own policy is the usual reason the client cannot be used
/// there, so each of these says what happened and who can do something about
/// it, in words that need no knowledge of OAuth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    AdminConsentRequired,
    AppBlocked,
    ConditionalAccess,
    DeviceNotCompliant,
    MfaRequired,
    AccountDisabled,
    WrongAccountType,
    Declined,
    /// The saved sign-in is no longer good and the person must sign in again.
    SignInExpired,
    TimedOut,
    Other,
}

impl Problem {
    pub fn code(self) -> &'static str {
        match self {
            Self::AdminConsentRequired => "admin_consent_required",
            Self::AppBlocked => "app_blocked",
            Self::ConditionalAccess => "conditional_access",
            Self::DeviceNotCompliant => "device_not_compliant",
            Self::MfaRequired => "mfa_required",
            Self::AccountDisabled => "account_disabled",
            Self::WrongAccountType => "wrong_account_type",
            Self::Declined => "declined",
            Self::SignInExpired => "sign_in_expired",
            Self::TimedOut => "timed_out",
            Self::Other => "sign_in_failed",
        }
    }

    pub fn explain(self) -> &'static str {
        match self {
            Self::AdminConsentRequired => {
                "Your organisation needs an administrator to approve Brolga before anyone can use it. \
                 Ask your IT administrator to approve the app for your organisation, then sign in again."
            }
            Self::AppBlocked => {
                "Your organisation does not allow Brolga, or has not set it up. Ask your IT administrator to allow the \
                 app, or to give you your organisation's own client ID to enter under Advanced."
            }
            Self::ConditionalAccess => {
                "Your organisation's access rules blocked this sign-in. They may limit which apps, places or networks \
                 can reach your mail. Your IT administrator can tell you which rule applied."
            }
            Self::DeviceNotCompliant => {
                "Your organisation only allows sign-in from devices it manages, and this computer is not one of them. \
                 Ask your IT administrator whether this device can be enrolled."
            }
            Self::MfaRequired => {
                "Your organisation needs you to confirm it is you with a second step. Sign in again and complete the \
                 extra verification when asked."
            }
            Self::AccountDisabled => {
                "This account has been disabled. Ask your IT administrator to re-enable it."
            }
            Self::WrongAccountType => {
                "That account is not a work or school account in this organisation. Brolga works with Microsoft 365 \
                 work and school accounts; personal accounts are not supported yet."
            }
            Self::Declined => {
                "The sign-in was cancelled, or the permissions Brolga asked for were not accepted."
            }
            Self::SignInExpired => {
                "Your sign-in has expired or was revoked. Sign in again to carry on."
            }
            Self::TimedOut => {
                "The sign-in was not finished in time. Start again when you are ready."
            }
            Self::Other => "The sign-in did not work.",
        }
    }

    /// Sort a refusal from the identity platform by its OAuth `error` and its
    /// numeric codes. The codes are those in "Microsoft Entra authentication
    /// and authorization error codes" (SOURCES.md); a code not listed there is
    /// not guessed at.
    pub fn classify(error: &str, codes: &[u32]) -> Self {
        for code in codes {
            let problem = match code {
                // AdminConsentRequired
                90094 => Self::AdminConsentRequired,
                // Not found in the tenant, disabled, user not assigned to it, or not an approved app.
                700016 | 7000112 | 50105 | 53002 => Self::AppBlocked,
                // BlockedByConditionalAccess, and an external challenge not satisfied.
                53003 | 50158 => Self::ConditionalAccess,
                // DeviceNotCompliant, DeviceNotDomainJoined, DeviceAuthenticationRequired.
                53000 | 53001 | 50097 => Self::DeviceNotCompliant,
                // Strong authentication required, enrolment required, or expired.
                50076 | 50079 | 50078 => Self::MfaRequired,
                50057 => Self::AccountDisabled,
                // UserUnauthorized: the account is from another identity provider or tenant.
                50020 => Self::WrongAccountType,
                // UserDeclinedConsent.
                65004 => Self::Declined,
                // Grants revoked or expired: password change, inactivity, sign-in frequency.
                50173 | 700082 | 70008 | 50089 | 70043 => Self::SignInExpired,
                _ => continue,
            };
            return problem;
        }

        match error {
            "access_denied" | "authorization_declined" => Self::Declined,
            "expired_token" => Self::TimedOut,
            "invalid_grant" | "interaction_required" | "login_required" | "consent_required" => {
                Self::SignInExpired
            }
            _ => Self::Other,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The identity platform said no. `description` is its own wording, kept
    /// for the person to pass to their administrator and never logged.
    #[error("{}", .problem.explain())]
    Refused {
        problem: Problem,
        error: String,
        codes: Vec<u32>,
        description: String,
    },
    #[error("{}", Problem::TimedOut.explain())]
    TimedOut,
    #[error("The sign-in service could not be reached. Check your connection and try again.")]
    Network(#[source] reqwest::Error),
    #[error("The sign-in service sent an answer Brolga could not read: {0}")]
    Protocol(String),
    #[error("The browser could not hand the sign-in back to Brolga: {0}")]
    Loopback(#[source] std::io::Error),
    #[error(
        "This computer could not supply random numbers, so a sign-in cannot be started safely: {0}"
    )]
    Random(String),
}

impl AuthError {
    pub fn problem(&self) -> Problem {
        match self {
            Self::Refused { problem, .. } => *problem,
            Self::TimedOut => Problem::TimedOut,
            _ => Problem::Other,
        }
    }

    pub(crate) fn refused(error: &str, codes: Vec<u32>, description: String) -> Self {
        Self::Refused {
            problem: Problem::classify(error, &codes),
            error: error.to_owned(),
            codes,
            description,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_documented_code_lands_on_its_problem() {
        let cases: &[(u32, Problem)] = &[
            (90094, Problem::AdminConsentRequired),
            (700016, Problem::AppBlocked),
            (7000112, Problem::AppBlocked),
            (53003, Problem::ConditionalAccess),
            (53000, Problem::DeviceNotCompliant),
            (50076, Problem::MfaRequired),
            (50079, Problem::MfaRequired),
            (50057, Problem::AccountDisabled),
            (50020, Problem::WrongAccountType),
            (65004, Problem::Declined),
            (50173, Problem::SignInExpired),
            (700082, Problem::SignInExpired),
        ];

        for (code, problem) in cases {
            assert_eq!(
                Problem::classify("invalid_grant", &[*code]),
                *problem,
                "AADSTS{code}"
            );
        }
    }

    #[test]
    fn a_code_that_says_more_than_the_error_wins() {
        // A revoked refresh token and a blocked one both arrive as invalid_grant.
        assert_eq!(
            Problem::classify("invalid_grant", &[53003]),
            Problem::ConditionalAccess
        );
        assert_eq!(
            Problem::classify("invalid_grant", &[]),
            Problem::SignInExpired
        );
        assert_eq!(
            Problem::classify("invalid_grant", &[999_999]),
            Problem::SignInExpired
        );
    }

    #[test]
    fn an_unknown_refusal_is_not_dressed_up_as_a_known_one() {
        assert_eq!(Problem::classify("server_error", &[123]), Problem::Other);
    }

    #[test]
    fn every_problem_explains_itself_without_jargon() {
        let all = [
            Problem::AdminConsentRequired,
            Problem::AppBlocked,
            Problem::ConditionalAccess,
            Problem::DeviceNotCompliant,
            Problem::MfaRequired,
            Problem::AccountDisabled,
            Problem::WrongAccountType,
            Problem::Declined,
            Problem::SignInExpired,
            Problem::TimedOut,
            Problem::Other,
        ];

        for problem in all {
            let text = problem.explain();
            assert!(text.len() > 20, "{problem:?}");
            for jargon in ["AADSTS", "OAuth", "token", "grant", "scope"] {
                assert!(!text.contains(jargon), "{problem:?} says `{jargon}`");
            }
        }
    }
}
