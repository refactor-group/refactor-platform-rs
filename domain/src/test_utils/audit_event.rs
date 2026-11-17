//! An audit recorder for tests that keeps events in memory.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sea_orm::DatabaseTransaction;

use crate::audit_event::{NewEvent, Recorder};
use crate::error::Error;

/// Keeps every recorded event in memory, in order, and never touches the database.
#[derive(Debug, Clone, Default)]
pub struct MemoryRecorder {
    events: Arc<Mutex<Vec<NewEvent>>>,
}

impl MemoryRecorder {
    /// The events recorded so far, oldest first.
    pub fn events(&self) -> Vec<NewEvent> {
        self.events
            .lock()
            .expect("the recorder's lock is not poisoned")
            .clone()
    }
}

#[async_trait]
impl Recorder for MemoryRecorder {
    async fn record(&self, _txn: &DatabaseTransaction, event: NewEvent) -> Result<(), Error> {
        self.events
            .lock()
            .expect("the recorder's lock is not poisoned")
            .push(event);
        Ok(())
    }
}
