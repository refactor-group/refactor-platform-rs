use crate::coaching_relationships::Model;
use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::Id;
use entity_api::coaching_relationship::notify_member_ids;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::user_role::retain_organization_members;
use entity_api::{coaching_relationships, query};
use sea_orm::{ConnectionTrait, DatabaseConnection, TransactionTrait};

pub use entity_api::coaching_relationship::{
    create, find_by_coach_and_organization, find_by_id, find_by_organization_with_user_names,
    find_by_user, find_by_user_and_organization, find_by_user_and_organization_with_user_names,
    find_by_user_id_with_user_names, find_coach_and_coachee, get_relationship_with_user_names,
    is_coach_of, CoachingRelationshipWithUserNames, Membership, RoleFilterable, WithMembership,
};

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<coaching_relationships::Column>,
{
    let coaching_relationships =
        query::find_by::<coaching_relationships::Entity, coaching_relationships::Column, P>(
            db, params,
        )
        .await?;
    Ok(coaching_relationships)
}

/// Finds coaching relationships for a user within an organization, respecting role-based access.
///
/// - SuperAdmins (global role with organization_id = NULL) see all relationships in the organization
/// - Admins (organization-specific role) see all relationships in their organization
/// - Regular users see only relationships where they are the coach or coachee
///
/// This function uses a database transaction to prevent Time-of-Check to Time-of-Use (TOCTOU)
/// vulnerabilities by ensuring the role check and data retrieval happen atomically within
/// a consistent database snapshot.
pub async fn find_by_organization_for_user_with_user_names(
    db: &DatabaseConnection,
    user_id: crate::Id,
    organization_id: crate::Id,
) -> Result<Vec<CoachingRelationshipWithUserNames>, Error> {
    // Begin transaction to ensure atomicity and prevent TOCTOU vulnerabilities
    let txn = db.begin().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    // Check if user has admin access using entity_api layer
    let is_admin = entity_api::user::has_admin_access(&txn, user_id, organization_id)
        .await
        .map_err(|e| Error {
            source: Some(Box::new(e)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::DbTransaction,
            )),
        })?;

    let coaching_relationships = if is_admin {
        // Admin users see all relationships in the organization
        find_by_organization_with_user_names(&txn, organization_id).await?
    } else {
        // Regular users see only relationships they're associated with (as coach or coachee)
        find_by_user_and_organization_with_user_names(&txn, user_id, organization_id).await?
    };

    // Commit transaction
    txn.commit().await.map_err(|e| Error {
        source: Some(Box::new(e)),
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::DbTransaction,
        )),
    })?;

    Ok(coaching_relationships)
}

/// The relationship's participants still in its organization, or a 409 when the coachee
/// is not one of them: anything sent on the relationship would not reach them.
pub(crate) async fn ensure_coachee_is_member(
    db: &impl ConnectionTrait,
    relationship: &Model,
) -> Result<Vec<Id>, Error> {
    let members = notify_member_ids(db, relationship).await?;
    match members.contains(&relationship.coachee_id) {
        true => Ok(members),
        false => Err(not_a_member(
            "Your coachee is no longer a member of this organization. Re-add them to the \
             organization first.",
            NotAMember::Coachee,
            relationship.organization_id,
            relationship.coachee_id,
        )),
    }
}

/// A 409 for the first of `assignee_ids`, in order, no longer in the relationship's
/// organization.
pub async fn ensure_assignees_are_members(
    db: &impl ConnectionTrait,
    relationship: &Model,
    assignee_ids: &[Id],
) -> Result<(), Error> {
    if assignee_ids.is_empty() {
        return Ok(());
    }
    let members =
        retain_organization_members(db, assignee_ids, relationship.organization_id).await?;
    assignee_ids
        .iter()
        .find(|id| !members.contains(id))
        .map_or(Ok(()), |&user_id| {
            Err(not_a_member(
                "An assignee is no longer a member of this organization; re-add them to the \
                 organization first.",
                NotAMember::Assignee,
                relationship.organization_id,
                user_id,
            ))
        })
}

/// Who was refused for no longer belonging to the organization.
#[derive(Clone, Copy, Debug)]
pub(crate) enum NotAMember {
    Coachee,
    Assignee,
}

impl NotAMember {
    fn reason(self) -> &'static str {
        match self {
            Self::Coachee => "coachee_not_a_member",
            Self::Assignee => "assignee_not_a_member",
        }
    }
}

/// A 409 for a user who no longer belongs to the organization, carrying a
/// machine-readable `reason`, the organization, and the user.
pub(crate) fn not_a_member(
    message: &str,
    who: NotAMember,
    organization_id: Id,
    user_id: Id,
) -> Error {
    Error::conflict(
        message,
        serde_json::json!({
            "reason": who.reason(),
            "organization_id": organization_id,
            "user_id": user_id,
        }),
    )
}

#[cfg(test)]
#[path = "coaching_relationship_sqlite_tests.rs"]
mod sqlite_tests;

#[cfg(test)]
#[path = "coaching_relationship_people_sqlite_tests.rs"]
mod people_sqlite_tests;

#[cfg(test)]
#[path = "coaching_relationship_membership_sqlite_tests.rs"]
mod membership_sqlite_tests;
