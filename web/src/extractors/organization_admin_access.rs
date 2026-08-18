use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::{request::Parts, StatusCode},
};
use domain::error::{DomainErrorKind, EntityErrorKind, Error as DomainError, InternalErrorKind};
use domain::{organization as OrganizationApi, organizations, users};
use log::*;

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, parse_path_id_from_parts, RejectionType},
    AppState,
};

/// Authorizes an administrator of the organization named by `:organization_id`.
///
/// Passes for a global SuperAdmin (`SuperAdmin` with no organization), or for a
/// user holding `Admin` in that specific organization. Carries the organization
/// and the authenticated user so handlers need not resolve either again.
pub(crate) struct OrganizationAdminAccess {
    pub organization: organizations::Model,
    pub authenticated_user: users::Model,
}

#[async_trait]
impl<S> FromRequestParts<S> for OrganizationAdminAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let organization_id = parse_path_id_from_parts(parts, "organization_id").await?;

        let AuthenticatedUser(authenticated_user) =
            AuthenticatedUser::from_request_parts(parts, &state).await?;

        // Evaluated in memory against the roles `AuthenticatedUser` already hydrated,
        // so this extractor costs no extra connection.
        let is_organization_admin = authenticated_user.roles.iter().any(|role| {
            (role.role == users::Role::SuperAdmin && role.organization_id.is_none())
                || (role.role == users::Role::Admin
                    && role.organization_id == Some(organization_id))
        });

        // Precedes the lookup so a non-admin cannot probe which organizations exist.
        if !is_organization_admin {
            return Err((
                StatusCode::FORBIDDEN,
                "You are not an administrator of the organization".to_string(),
            ));
        }

        let organization = OrganizationApi::find_by_id(state.db_conn_ref(), organization_id)
            .await
            .map_err(|err| {
                let domain_err: DomainError = err.into();
                match domain_err.error_kind {
                    DomainErrorKind::Internal(InternalErrorKind::Entity(
                        EntityErrorKind::NotFound,
                    )) => (
                        StatusCode::NOT_FOUND,
                        format!("Organization {organization_id} not found"),
                    ),
                    _ => {
                        error!(
                            "find_by_id({organization_id:?}) failed while verifying organization existence: {domain_err:?}"
                        );
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Failed to verify organization existence".to_string(),
                        )
                    }
                }
            })?;

        Ok(OrganizationAdminAccess {
            organization,
            authenticated_user,
        })
    }
}
