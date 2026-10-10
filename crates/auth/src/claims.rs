//! Claims challenges: how an API says a token is no longer enough and the
//! person must sign in again, for instance after their sign-in was revoked or
//! their location changed (Continuous Access Evaluation).

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};

/// The claims a 401's `WWW-Authenticate` header asks for, ready to pass as
/// the `claims` parameter of a new sign-in. Nothing if the header is not a
/// claims challenge.
///
/// The header carries the claims as base64; the sign-in wants the JSON.
pub fn claims_challenge(www_authenticate: &str) -> Option<String> {
    if parameter(www_authenticate, "error")? != "insufficient_claims" {
        return None;
    }

    let encoded = parameter(www_authenticate, "claims")?;
    let decoded = STANDARD
        .decode(&encoded)
        .or_else(|_| STANDARD_NO_PAD.decode(encoded.trim_end_matches('=')))
        .ok()?;
    let json = String::from_utf8(decoded).ok()?;

    // Only pass on what is really a JSON object.
    serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&json).ok()?;

    Some(json)
}

/// One `name="value"` parameter of the header. Parameters come in any order.
fn parameter(header: &str, name: &str) -> Option<String> {
    let mut rest = header;

    while let Some(at) = rest.find(name) {
        let before_ok = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| c == ' ' || c == ',');
        let after = rest[at + name.len()..].trim_start();

        if before_ok && let Some(value) = after.strip_prefix('=') {
            let value = value.trim_start();
            return Some(match value.strip_prefix('"') {
                Some(quoted) => quoted.split('"').next()?.to_owned(),
                None => value.split([',', ' ']).next()?.to_owned(),
            });
        }

        rest = &rest[at + name.len()..];
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_challenge_is_read() {
        // The example from "Claims challenges, claims requests and client capabilities" (SOURCES.md).
        let header = "Bearer realm=\"\", authorization_uri=\"https://login.microsoftonline.com/common/oauth2/authorize\", \
                      error=\"insufficient_claims\", \
                      claims=\"eyJhY2Nlc3NfdG9rZW4iOnsiYWNycyI6eyJlc3NlbnRpYWwiOnRydWUsInZhbHVlIjoiY3AxIn19fQ==\"";

        assert_eq!(
            claims_challenge(header).as_deref(),
            Some("{\"access_token\":{\"acrs\":{\"essential\":true,\"value\":\"cp1\"}}}")
        );
    }

    #[test]
    fn parameters_may_come_in_any_order() {
        let header = "Bearer claims=\"e30=\", error=\"insufficient_claims\"";

        assert_eq!(claims_challenge(header).as_deref(), Some("{}"));
    }

    #[test]
    fn an_ordinary_401_is_not_a_challenge() {
        for header in [
            "Bearer realm=\"\"",
            "Bearer error=\"invalid_token\", claims=\"e30=\"",
            "Bearer error=\"insufficient_claims\"",
            "Bearer error=\"insufficient_claims\", claims=\"not base64!\"",
            "Bearer error=\"insufficient_claims\", claims=\"bm90IGpzb24=\"",
            "",
        ] {
            assert_eq!(claims_challenge(header), None, "{header}");
        }
    }
}
