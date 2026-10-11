use std::time::Duration;

/// How patient to be with a server that says to slow down.
///
/// "Microsoft Graph throttling guidance" (SOURCES.md): wait the number of
/// seconds in `Retry-After`, and where there is none, back off exponentially.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// How many times to send a request in all before giving up.
    pub attempts: u32,
    /// The first wait where the server names none; it doubles each time.
    pub base: Duration,
    /// The longest any one wait may be, whatever the server asks.
    pub max_wait: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 6,
            base: Duration::from_millis(500),
            max_wait: Duration::from_secs(120),
        }
    }
}

impl RetryPolicy {
    /// For tests: as persistent, and not slow.
    pub fn impatient() -> Self {
        Self {
            attempts: 6,
            base: Duration::from_millis(1),
            max_wait: Duration::from_millis(5),
        }
    }

    /// How long to wait before attempt `attempt + 1`, having been told
    /// `retry_after` seconds, or told nothing.
    pub(crate) fn wait(&self, attempt: u32, retry_after: Option<u64>) -> Duration {
        let wait = match retry_after {
            Some(seconds) => Duration::from_secs(seconds),
            None => {
                let doubled = self.base.saturating_mul(2u32.saturating_pow(attempt));
                // Full jitter, so that clients throttled together do not all
                // come back together.
                doubled.mul_f64(0.5 + random_fraction() / 2.0)
            }
        };

        wait.min(self.max_wait)
    }
}

/// A number from 0 up to 1. If the system has no randomness to give, the
/// wait is simply not jittered.
fn random_fraction() -> f64 {
    let mut bytes = [0u8; 4];
    match getrandom::fill(&mut bytes) {
        Ok(()) => f64::from(u32::from_le_bytes(bytes)) / f64::from(u32::MAX),
        Err(_) => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_s_own_wait_is_honoured_up_to_the_cap() {
        let policy = RetryPolicy::default();

        assert_eq!(policy.wait(0, Some(10)), Duration::from_secs(10));
        assert_eq!(policy.wait(3, Some(10)), Duration::from_secs(10));
        assert_eq!(policy.wait(0, Some(100_000)), policy.max_wait);
    }

    #[test]
    fn with_no_wait_named_each_try_waits_longer_within_bounds() {
        let policy = RetryPolicy::default();

        for attempt in 0..6 {
            let full = policy.base * 2u32.pow(attempt);
            for _ in 0..50 {
                let wait = policy.wait(attempt, None);
                assert!(
                    wait >= full / 2 && wait <= full,
                    "attempt {attempt}: {wait:?}"
                );
            }
        }
        assert!(policy.wait(40, None) <= policy.max_wait);
    }
}
