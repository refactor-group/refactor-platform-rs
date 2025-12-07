use crate::coaching_relationships::Model;
use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{coaching_relationships, query, user_roles, Role};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

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
pub async fn find_by_organization_for_user_with_user_names(
    db: &DatabaseConnection,
    user_id: crate::Id,
    organization_id: crate::Id,
) -> Result<Vec<CoachingRelationshipWithUserNames>, Error> {
    // Check if user is a super admin (has role = 'super_admin' with organization_id = NULL)
    let is_super_admin = user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(user_roles::Column::Role.eq(Role::SuperAdmin))
        .filter(user_roles::Column::OrganizationId.is_null())
        .one(db)
        .await
        .map_err(|e| Error {
            source: Some(Box::new(e)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::DbTransaction,
            )),
        })?
        .is_some();

    // Check if user is an admin for this specific organization
    let is_org_admin = user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(user_roles::Column::Role.eq(Role::Admin))
        .filter(user_roles::Column::OrganizationId.eq(organization_id))
        .one(db)
        .await
        .map_err(|e| Error {
            source: Some(Box::new(e)),
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::DbTransaction,
            )),
        })?
        .is_some();

    let coaching_relationships = if is_super_admin || is_org_admin {
        // Admin users see all relationships in the organization
        find_by_organization_with_user_names(db, organization_id).await?
    } else {
        // Regular users see only relationships they're associated with (as coach or coachee)
        find_by_user_and_organization_with_user_names(db, user_id, organization_id).await?
    };

    Ok(coaching_relationships)
}

#[cfg(test)]
mod tests {
    /// Test documentation for role-based access control in find_by_organization_for_user_with_user_names
    ///
    /// These tests document the expected behavior but require integration tests with a real database
    /// to execute, as CoachingRelationshipWithUserNames is a FromQueryResult type that cannot be mocked
    /// with SeaORM's MockDatabase.
    ///
    /// Expected behaviors:
    ///
    /// 1. **Normal users** (no admin roles):
    ///    - Should only see relationships where they are the coach OR coachee
    ///    - Should NOT see other users' relationships in the same organization
    ///
    /// 2. **Organization admins** (Admin role for specific org):
    ///    - Should see ALL relationships within their organization
    ///    - Should see relationships even if they are not involved as coach/coachee
    ///    - When querying a different organization, should only see their own relationships (not admin there)
    ///
    /// 3. **Super admins** (SuperAdmin role with organization_id = NULL):
    ///    - Should see ALL relationships in ANY organization they query
    ///    - Have global access across all organizations
    ///
    /// 4. **Edge cases**:
    ///    - Users with no relationships should see an empty list
    ///    - Role checks happen at the database level for security
    ///
    /// To add integration tests, create tests in the web crate that:
    /// - Set up test database with users, roles, and relationships
    /// - Call the endpoint for each user type
    /// - Verify correct filtering based on role
    #[test]
    fn test_role_based_access_documentation() {
        // This test exists to document the expected behavior
        // See docstring above for test scenarios that should be verified
        // in integration tests
    }
}
