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

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_role_change_tests.rs"]
mod tests;
