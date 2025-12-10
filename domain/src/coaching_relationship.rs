use crate::coaching_relationships::Model;
use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{coaching_relationships, query, user_roles, Role};
use sea_orm::{ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter};

pub use entity_api::coaching_relationship::{
    create, find_by_id, find_by_organization_with_user_names, find_by_user,
    find_by_user_and_organization_with_user_names, get_relationship_with_user_names,
    CoachingRelationshipWithUserNames,
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
/// Role checks are combined into a single query to improve performance and reduce the risk
/// of race conditions between multiple separate queries.
pub async fn find_by_organization_for_user_with_user_names(
    db: &DatabaseConnection,
    user_id: crate::Id,
    organization_id: crate::Id,
) -> Result<Vec<CoachingRelationshipWithUserNames>, Error> {
    // Check if user is a super admin OR an admin for this organization in a single query
    let admin_role = user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(
            Condition::any()
                // SuperAdmin with organization_id = NULL
                .add(
                    Condition::all()
                        .add(user_roles::Column::Role.eq(Role::SuperAdmin))
                        .add(user_roles::Column::OrganizationId.is_null()),
                )
                // Admin for this specific organization
                .add(
                    Condition::all()
                        .add(user_roles::Column::Role.eq(Role::Admin))
                        .add(user_roles::Column::OrganizationId.eq(organization_id)),
                ),
        )
        .one(db)
        .await
        .map_err(|e| Error {
            source: Some(Box::new(e)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::DbTransaction,
            )),
        })?;

    let is_admin = admin_role.is_some();

    let coaching_relationships = if is_admin {
        // Admin users see all relationships in the organization
        find_by_organization_with_user_names(db, organization_id).await?
    } else {
        // Regular users see only relationships they're associated with (as coach or coachee)
        find_by_user_and_organization_with_user_names(db, user_id, organization_id).await?
    };

    Ok(coaching_relationships)
}
