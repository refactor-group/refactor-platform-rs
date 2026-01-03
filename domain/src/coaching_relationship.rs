use crate::coaching_relationships::Model;
use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{coaching_relationships, query};
use sea_orm::{DatabaseConnection, TransactionTrait};

pub use entity_api::coaching_relationship::{
    create, find_by_coach_id_with_user_names, find_by_coachee_id_with_user_names, find_by_id,
    find_by_organization_with_user_names, find_by_user,
    find_by_user_and_organization_with_user_names, get_relationship_with_user_names,
    get_roles_summary, CoachingRelationshipWithUserNames, RolesSummary,
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
