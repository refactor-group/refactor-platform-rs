//! Records changes to an organization in its audit log, inside the transaction that makes
//! them. See `docs/architecture/audit_log.md`.

use async_trait::async_trait;
use sea_orm::DatabaseTransaction;

use crate::error::Error;

pub use entity_api::audit_event::{verify_chain, ChainCheck, NewEvent};

/// Records audit events. A domain write takes one beside its `EventPublisher` and calls
/// `record` before committing, so a failed record rolls the change back.
#[async_trait]
pub trait Recorder: Send + Sync {
    /// Records `event` as part of `txn`.
    async fn record(&self, txn: &DatabaseTransaction, event: NewEvent) -> Result<(), Error>;
}

/// The production recorder: appends each event to the organization's chain in the database.
#[derive(Debug, Clone, Copy, Default)]
pub struct DatabaseRecorder;

#[async_trait]
impl Recorder for DatabaseRecorder {
    async fn record(&self, txn: &DatabaseTransaction, event: NewEvent) -> Result<(), Error> {
        entity_api::audit_event::append(txn, event).await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "audit_event_sqlite_tests.rs"]
mod sqlite_tests;
