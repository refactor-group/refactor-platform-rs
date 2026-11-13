use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
use domain::error::{DomainErrorKind, EntityErrorKind, Error as DomainError, InternalErrorKind};
use domain::{organization as OrganizationApi, user as UserApi, Id};

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, RejectionType},
    AppState,
};
use log::*;

/// Checks that the authenticated user is associated with the organization specified by `organization_id`
/// Passes if:
/// * User is a SuperAdmin (has `SuperAdmin` role with `organization_id = NULL`), OR
/// * User has any role in the specified organization
pub(crate) struct OrganizationMemberAccess(pub Id);

#[async_trait]
impl<S> FromRequestParts<S> for OrganizationMemberAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let Path(path_params) = Path::<HashMap<String, String>>::from_request_parts(parts, &state)
            .await
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Invalid path parameters".to_string(),
                )
            })?;

        let organization_id_str = path_params.get("organization_id").ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid organization id".to_string(),
            )
        })?;

        let organization_id = organization_id_str.parse::<Id>().map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid organization id".to_string(),
            )
        })?;

        let AuthenticatedUser(authenticated_user) =
            AuthenticatedUser::from_request_parts(parts, &state).await?;

        if let Err(err) = OrganizationApi::find_by_id(state.db_conn_ref(), organization_id).await {
            let domain_err: DomainError = err.into();
            return match domain_err.error_kind {
                DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound)) => {
                    Err((
                        StatusCode::NOT_FOUND,
                        format!("Organization {organization_id} not found"),
                    ))
                }
                _ => {
                    error!(
                        "find_by_id({organization_id:?}) failed while verifying organization existence: {domain_err:?}"
                    );
                    Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to verify organization existence".to_string(),
                    ))
                }
            };
        }

        // SuperAdmins have access to all organizations
        if authenticated_user.is_super_admin() {
            return Ok(OrganizationMemberAccess(organization_id));
        }

        let user_organization_role_exists = match UserApi::find_by_organization(
            state.db_conn_ref(),
            organization_id,
        )
        .await
        {
            Ok(users) => users.iter().any(|user| user.id == authenticated_user.id),
            Err(err) => {
                error!(
                        "find_by_organization({organization_id:?}) failed while verifying membership: {err:?}"
                    );
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to verify organization membership".to_string(),
                ));
            }
        };

        if !user_organization_role_exists {
            return Err((
                StatusCode::FORBIDDEN,
                "You are not authorized to access the organization".to_string(),
            ));
        }

        Ok(OrganizationMemberAccess(organization_id))
    }
}
