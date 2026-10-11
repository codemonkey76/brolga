//! The outbox: every change a user makes, as a queued, idempotent operation.
//!
//! A change is made on this computer first (`brolga-store` does that, and
//! queues it in the same transaction). This crate sends what is queued. It
//! can be stopped at any point and started again: a change stays queued
//! until the server has answered for it, and sending one twice leaves the
//! mailbox as sending it once does.

use std::collections::HashSet;
use std::sync::Arc;

use brolga_graph::{GraphClient, GraphError};
use brolga_store::{AccountDb, Op, StoreError};
use serde_json::json;
use tokio::sync::Mutex;

/// What became of the queue in one pass over it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Drained {
    /// The server has these.
    pub sent: u32,
    /// These could not be sent just now and are still queued.
    pub waiting: u32,
    /// The server will not take these. Each was taken back on this computer.
    pub refused: u32,
}

/// Sends one account's queued changes.
pub struct Outbox {
    db: Arc<AccountDb>,
    graph: Arc<GraphClient>,
    /// One pass at a time, so a change is not sent by two at once.
    draining: Mutex<()>,
}

impl Outbox {
    pub fn new(graph: Arc<GraphClient>, db: Arc<AccountDb>) -> Self {
        Self {
            db,
            graph,
            draining: Mutex::new(()),
        }
    }

    /// Send what is queued, oldest first. The changes to one message are
    /// sent in the order they were made: when one cannot be sent now, the
    /// later ones for that message wait behind it.
    pub async fn drain(&self) -> Result<Drained, StoreError> {
        let _one_at_a_time = self.draining.lock().await;
        let mut drained = Drained::default();
        let mut held_up: HashSet<String> = HashSet::new();

        for op in self.db.queued_ops()? {
            if held_up.contains(&op.message_id) {
                drained.waiting += 1;
                continue;
            }

            match self.send(&op).await {
                Ok(()) => {
                    self.db.op_sent(op.id)?;
                    drained.sent += 1;
                }
                // The message is gone from the server. There is nothing left
                // to change, and the next sync takes it from here too.
                Err(GraphError::Refused { status: 404, .. }) => {
                    self.db.op_sent(op.id)?;
                    drained.sent += 1;
                }
                Err(error) if will_pass(&error) => {
                    self.db.op_waits(op.id, &error.to_string())?;
                    held_up.insert(op.message_id);
                    drained.waiting += 1;
                }
                Err(error) => {
                    self.db.op_refused(op.id, &error.to_string())?;
                    drained.refused += 1;
                }
            }
        }

        Ok(drained)
    }

    async fn send(&self, op: &Op) -> Result<(), GraphError> {
        match op.kind.as_str() {
            // Setting a property twice leaves it as setting it once does.
            "is_read" => {
                let changes = json!({ "isRead": op.value == "1" });
                self.graph.update_message(&op.message_id, &changes).await
            }
            "flag_status" => {
                let changes = json!({ "flag": { "flagStatus": op.value } });
                self.graph.update_message(&op.message_id, &changes).await
            }
            "folder_id" => self.move_message(op).await,
            other => Err(GraphError::Decode(format!(
                "a queued change of a kind this version does not know: {other}"
            ))),
        }
    }

    /// A move is not safe to send twice blindly. When an earlier try may
    /// have reached the server, the message's folder is looked at first,
    /// and the move is sent only if it has not already been made.
    async fn move_message(&self, op: &Op) -> Result<(), GraphError> {
        if op.attempts > 0
            && self.graph.message_folder(&op.message_id).await?.as_deref()
                == Some(op.value.as_str())
        {
            return Ok(());
        }

        self.graph.move_message(&op.message_id, &op.value).await
    }
}

/// Whether the same request may well succeed later: the network is down,
/// the service is busy, or the account has to sign in again first.
fn will_pass(error: &GraphError) -> bool {
    error.is_transient()
        || matches!(
            error,
            GraphError::Token(_) | GraphError::Unauthorized { .. }
        )
}
