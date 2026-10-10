//! Proof Key for Code Exchange, RFC 7636.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use crate::AuthError;

/// Random bytes as unpadded base64url: unguessable, and safe in a URL.
pub(crate) fn random_token(bytes: usize) -> Result<String, AuthError> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).map_err(|e| AuthError::Random(e.to_string()))?;

    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

/// A code verifier: 32 random bytes, which is the 43 characters RFC 7636
/// section 4.1 recommends.
pub(crate) fn verifier() -> Result<String, AuthError> {
    random_token(32)
}

/// The S256 challenge for a verifier, RFC 7636 section 4.2.
pub(crate) fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_matches_the_rfc_example() {
        // RFC 7636, appendix B.
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn a_verifier_is_43_url_safe_characters_and_never_the_same_twice() {
        let first = verifier().unwrap();
        let second = verifier().unwrap();

        assert_eq!(first.len(), 43);
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_ne!(first, second);
    }
}
