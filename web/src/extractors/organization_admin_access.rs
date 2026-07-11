use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};

use domain::error::{DomainErrorKind, EntityErrorKind, Error as DomainError, InternalErrorKind};
use domain::{organization as OrganizationApi, Id};

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, parse_path_id, RejectionType},
    AppState,
};
use log::*;

/// Checks that the authenticated user is associated with the organization specified by `organization_id`
/// Passes if:
/// * User is a SuperAdmin (has `SuperAdmin` role with `organization_id = NULL`), OR
/// * User has an admin role in the specified organization
pub(crate) struct OrganizationAdminAccess(pub Id);

#[async_trait]
impl<S> FromRequestParts<S> for OrganizationAdminAccess
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

        let organization_id = parse_path_id(&path_params, "organization_id")?;

        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, &state).await?;

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

        user.roles
            .iter()
            .any(|r| {
                r.role == domain::users::Role::SuperAdmin && r.organization_id.is_none()
                    || r.role == domain::users::Role::Admin
                        && r.organization_id == Some(organization_id)
            })
            .then_some(OrganizationAdminAccess(organization_id))
            .ok_or((StatusCode::UNAUTHORIZED, "UNAUTHORIZED".to_string()))
    }
}
