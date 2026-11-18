//! Records changes to an organization in its audit log, inside the transaction that makes
//! them. See `docs/architecture/audit_log.md`.

use async_trait::async_trait;
use sea_orm::{ActiveEnum, DatabaseTransaction};
use serde_json::{json, Value};

use crate::audit_events::Action;
use crate::coaching_relationships;
use crate::error::Error;
use crate::user::Role;
use crate::{Actor, Id};

pub use entity_api::audit_event::{verify_chain, ChainCheck, NewEvent};

/// Records audit events. A domain write takes one beside its `EventPublisher` and records
/// after its last other write, immediately before committing, so a failed record rolls the
/// change back and the audit lock is always the last lock its transaction takes.
#[async_trait]
pub trait Recorder: Send + Sync {
    /// Records `event` as part of `txn`.
    async fn record(&self, txn: &DatabaseTransaction, event: NewEvent) -> Result<(), Error>;

    /// Records `events` as part of `txn`, ordered by organization and otherwise in the order
    /// given, so transactions touching several organizations take their audit locks in one
    /// order.
    async fn record_all(
        &self,
        txn: &DatabaseTransaction,
        mut events: Vec<NewEvent>,
    ) -> Result<(), Error> {
        events.sort_by_key(|event| event.organization_id);
        for event in events {
            self.record(txn, event).await?;
        }
        Ok(())
    }
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

/// `actor` created `relationship`.
pub fn relationship_created(
    actor: Actor,
    relationship: &coaching_relationships::Model,
) -> NewEvent {
    relationship_event(actor, Action::RelationshipCreated, relationship)
}

/// `actor` deleted `relationship`.
pub fn relationship_deleted(
    actor: Actor,
    relationship: &coaching_relationships::Model,
) -> NewEvent {
    relationship_event(actor, Action::RelationshipDeleted, relationship)
}

/// `actor` gave `user_id` a `role` in the organization.
pub fn member_added(actor: Actor, organization_id: Id, user_id: Id, role: Role) -> NewEvent {
    member_event(
        actor,
        Action::MemberAdded,
        organization_id,
        user_id,
        json!({ "role": role.to_value() }),
    )
}

/// `actor` removed `user_id` from the organization.
pub fn member_removed(actor: Actor, organization_id: Id, user_id: Id) -> NewEvent {
    member_event(
        actor,
        Action::MemberRemoved,
        organization_id,
        user_id,
        json!({}),
    )
}

/// `actor` changed `user_id`'s role in the organization from `from` to `to`.
pub fn member_role_changed(
    actor: Actor,
    organization_id: Id,
    user_id: Id,
    from: Role,
    to: Role,
) -> NewEvent {
    member_event(
        actor,
        Action::MemberRoleChanged,
        organization_id,
        user_id,
        json!({ "from": from.to_value(), "to": to.to_value() }),
    )
}

/// An event about `relationship`, in its organization, naming its kind and who holds each seat.
fn relationship_event(
    actor: Actor,
    action: Action,
    relationship: &coaching_relationships::Model,
) -> NewEvent {
    NewEvent {
        organization_id: relationship.organization_id,
        actor: Some(actor),
        action,
        subject_id: relationship.id,
        details: json!({
            "kind": relationship.kind.to_value(),
            "coach_id": relationship.coach_id,
            "coachee_id": relationship.coachee_id,
        }),
    }
}

/// An event about `user_id`'s membership of the organization.
fn member_event(
    actor: Actor,
    action: Action,
    organization_id: Id,
    user_id: Id,
    details: Value,
) -> NewEvent {
    NewEvent {
        organization_id,
        actor: Some(actor),
        action,
        subject_id: user_id,
        details,
    }
}

#[cfg(test)]
#[path = "audit_event_sqlite_tests.rs"]
mod sqlite_tests;
