use super::error::Error;

use super::actor::Actor;
use entity::roles::Role;
use entity::user_role_changes::{ActiveModel, Column, Entity};
use entity::user_roles;
use entity::Id;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect, Set,
};

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
pub async fn was_member_of_administered_organization(
    db: &impl ConnectionTrait,
    requester_id: Id,
    target_user_id: Id,
) -> Result<bool, Error> {
    let administered_organization_ids = user_roles::Entity::find()
        .select_only()
        .column(user_roles::Column::OrganizationId)
        .filter(user_roles::Column::UserId.eq(requester_id))
        .filter(user_roles::Column::Role.eq(Role::Admin))
        .filter(user_roles::Column::OrganizationId.is_not_null())
        .into_tuple::<Option<Id>>()
        .all(db)
        .await?
        .into_iter()
        .flatten()
        .collect::<Vec<Id>>();

    // Runs even when the set is empty: an empty `is_in` matches nothing, and skipping
    // the probe would make the query count reveal the answer.
    Ok(Entity::find()
        .select_only()
        .column(Column::Id)
        .filter(Column::TargetUserId.eq(target_user_id))
        .filter(Column::OrganizationId.is_in(administered_organization_ids))
        .into_tuple::<Id>()
        .one(db)
        .await?
        .is_some())
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "user_role_change_tests.rs"]
mod tests;
