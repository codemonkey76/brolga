#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("Microsoft 365 could not be reached. Check your connection.")]
    Network(#[source] reqwest::Error),
    /// Still throttled or unavailable after every retry allowed.
    #[error("Microsoft 365 is busy and asked Brolga to wait. It will try again shortly.")]
    Busy { status: u16 },
    /// The saved place in a delta sync is no longer accepted, and the
    /// resource must be synced again from the start.
    #[error("Microsoft 365 asked for a fresh sync of this folder.")]
    SyncStateGone,
    /// The token was refused. `claims` carries a claims challenge where
    /// there was one, for the person to sign in again against.
    #[error("Microsoft 365 did not accept the sign-in for this request.")]
    Unauthorized { claims: Option<String> },
    #[error("Microsoft 365 refused the request ({status} {code}): {message}")]
    Refused {
        status: u16,
        code: String,
        message: String,
    },
    #[error("Microsoft 365 sent an answer Brolga could not read: {0}")]
    Decode(String),
    /// No token could be had for the account: it needs to sign in, or is offline.
    #[error("{0}")]
    Token(String),
}

impl GraphError {
    /// True when trying again later, unchanged, may well work.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Busy { .. })
    }

    /// True when the request was certainly not carried out: it never
    /// reached Microsoft 365, or was turned away there before anything was
    /// done. A request that fails any other way may have been carried out
    /// all the same, and one that must happen only once is not to be made
    /// again on the strength of the failure.
    pub fn not_carried_out(&self) -> bool {
        match self {
            // No connection was made, so nothing was sent along one.
            Self::Network(error) => error.is_connect(),
            // Throttled ("Microsoft Graph throttling guidance", SOURCES.md):
            // the request fails and is to be made again. Any other busy
            // answer is a failure on the server's side, which says nothing
            // of how far the request got.
            Self::Busy { status } => *status == 429,
            Self::Unauthorized { .. } | Self::Refused { .. } | Self::Token(_) => true,
            Self::SyncStateGone | Self::Decode(_) => false,
        }
    }
}
