//! What differs from one operating system to the next: where secrets are
//! kept, where files go, and how a link is handed to the browser.

use std::path::PathBuf;

use brolga_auth::{SecretError, SecretStore};

/// The service name every secret is filed under in the system's store.
const SERVICE: &str = "brolga";

/// The operating system's own secret store: the Secret Service on Linux,
/// Credential Manager on Windows, the Keychain on macOS.
pub struct SystemSecrets;

impl SystemSecrets {
    fn entry(key: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(SERVICE, key).map_err(unavailable)
    }
}

impl SecretStore for SystemSecrets {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        match Self::entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(unavailable(e)),
        }
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), SecretError> {
        Self::entry(key)?.set_password(secret).map_err(unavailable)
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(unavailable(e)),
        }
    }
}

fn unavailable(error: keyring::Error) -> SecretError {
    SecretError::Unavailable(error.to_string())
}

/// Where Brolga keeps its data for this user: the XDG data directory on
/// Linux, and the usual application data folder on Windows and macOS.
pub fn data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("io.github", "codemonkey76", "Brolga")
        .map(|dirs| dirs.data_dir().to_path_buf())
}

/// Hand a link to the system's default browser.
pub fn open_in_browser(url: &str) -> std::io::Result<()> {
    open::that_detached(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against the real secret store of whoever runs it, so it is not part
    /// of the ordinary run: `cargo test -p brolga-platform -- --ignored`.
    #[test]
    #[ignore = "writes to, and cleans up after itself in, this user's real secret store"]
    fn a_secret_goes_into_the_system_store_and_comes_back_out() {
        let key = format!("test.{}", std::process::id());
        let store = SystemSecrets;

        assert_eq!(store.get(&key).unwrap(), None);
        store.set(&key, "first").unwrap();
        store.set(&key, "second").unwrap();
        assert_eq!(store.get(&key).unwrap().as_deref(), Some("second"));
        store.delete(&key).unwrap();
        assert_eq!(store.get(&key).unwrap(), None);
        // Forgetting what is not there is not an error.
        store.delete(&key).unwrap();
    }
}
