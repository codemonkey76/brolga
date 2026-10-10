use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("this computer's secret store is not available: {0}")]
    Unavailable(String),
}

/// Where refresh tokens live: the operating system's secret store, and
/// nowhere else. Never a file, never a log.
pub trait SecretStore: Send + Sync {
    /// The secret saved under `key`, or nothing if there is none.
    fn get(&self, key: &str) -> Result<Option<String>, SecretError>;
    fn set(&self, key: &str, secret: &str) -> Result<(), SecretError>;
    /// Forget the secret. Forgetting one that is not there is not an error.
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

/// A secret store that lasts as long as the process. For tests.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl MemorySecrets {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        Ok(self.map().get(key).cloned())
    }

    fn set(&self, key: &str, secret: &str) -> Result<(), SecretError> {
        self.map().insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        self.map().remove(key);
        Ok(())
    }
}
