use super::error::Error;

use super::actor::Actor;
use entity::roles::Role;
use entity::user_role_changes::ActiveModel;
use entity::Id;
use sea_orm::{ActiveModelTrait, ConnectionTrait, Set};

/// Appends one audit row describing a role change.
///
/// Crate-private on purpose: audit rows are written by the `user_role` mutations
/// themselves, so there is no way to change a role without recording it.
///
/// `previous_role` is `None` for a first grant, `new_role` is `None` for a
/// removal, and `organization_id` is `None` for a global SuperAdmin grant.
/// Takes a `ConnectionTrait` so it composes inside the caller's transaction,
/// keeping the audit row and the change it describes atomic.
///
/// `changed_at` is left to the column default so the database clock stamps it.
/// Application clocks skew across instances, and rows written in one transaction
/// then share a single timestamp, which is what a reader expects of one event.
pub(crate) async fn record(
    db: &impl ConnectionTrait,
    actor: Option<Actor>,
    target_user_id: Id,
    organization_id: Option<Id>,
    previous_role: Option<Role>,
    new_role: Option<Role>,
) -> Result<(), Error> {
    ActiveModel {
        actor_user_id: Set(actor.map(|actor| actor.id())),
        target_user_id: Set(target_user_id),
        organization_id: Set(organization_id),
        previous_role: Set(previous_role),
        new_role: Set(new_role),
        ..Default::default()
    }
    .insert(db)
    .await?;

    Ok(())
}

/// Whether `target_user_id` has any recorded role change in an organization that
/// `requester_id` administers. A prior grant or removal is proof the caller's
/// organization already knew this person, so surfacing them discloses nothing new.
///
/// Always issues exactly two queries, mirroring
/// `user_role::shares_administered_organization`, so a caller composing the two
/// keeps a constant query count. That is an anti-enumeration timing property, not
/// a performance note.
///
/// Only sees changes recorded since `user_role_changes` was created (2026-08-16).
/// Earlier removals left no row and are invisible here.
// Names kept for the later implementation; `todo!()` leaves them unread.
#[allow(unused_variables)]
pub async fn was_member_of_administered_organization(
    db: &impl ConnectionTrait,
    requester_id: Id,
    target_user_id: Id,
) -> Result<bool, Error> {
    todo!()
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_role_change_tests.rs"]
mod tests;
