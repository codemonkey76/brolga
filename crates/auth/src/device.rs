//! Signing in with a device code, for a machine with no browser of its own,
//! or one whose browser cannot be handed the sign-in.

use std::time::Duration;

use tokio::time::Instant;

use crate::token::life;
use crate::{AuthError, Registration, TokenClient, Tokens};

/// A sign-in that has been started and is waiting for the person to enter the
/// code on another device.
pub struct DeviceSignIn {
    /// The short code to type in.
    pub user_code: String,
    /// The page to type it in at.
    pub verification_uri: String,
    /// The identity platform's own instructions, ready to show.
    pub message: String,
    /// How long the code is good for.
    pub expires_in: Duration,
    registration: Registration,
    device_code: String,
    interval: Duration,
}

impl DeviceSignIn {
    pub async fn begin(
        client: &TokenClient,
        registration: &Registration,
        scopes: &[&str],
    ) -> Result<Self, AuthError> {
        let body = client.device_code(registration, scopes).await?;

        Ok(Self {
            user_code: body.user_code,
            verification_uri: body.verification_uri,
            message: body.message,
            expires_in: life(body.expires_in),
            registration: registration.clone(),
            device_code: body.device_code,
            // Never poll faster than once a second, whatever we are told,
            // nor so slowly that the code has run out before the first try.
            interval: Duration::from_secs(body.interval.clamp(1, 60)),
        })
    }

    /// Ask, no more often than we were told to, until the person has signed
    /// in, refused, or run out of time.
    pub async fn finish(self, client: &TokenClient) -> Result<Tokens, AuthError> {
        let deadline = Instant::now() + self.expires_in;
        let mut interval = self.interval;

        loop {
            tokio::time::sleep(interval).await;
            if Instant::now() >= deadline {
                return Err(AuthError::TimedOut);
            }

            match client
                .device_grant(&self.registration, &self.device_code)
                .await
            {
                Err(AuthError::Refused { error, .. }) if error == "authorization_pending" => {}
                // RFC 8628 section 3.5: on slow_down, wait five seconds longer from now on.
                Err(AuthError::Refused { error, .. }) if error == "slow_down" => {
                    interval += Duration::from_secs(5)
                }
                other => return other,
            }
        }
    }
}
