use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
use domain::{user as UserApi, users, Id};

use crate::{extractors::RejectionType, AppState};
use log::*;

/// Authorizes that the target user named by the `{user_id}` path segment is a
/// member of the `{organization_id}` named in the same path, yielding the loaded
/// target user model on success.
///
/// This closes the cross-tenant gap on user-scoped organization routes: the
/// caller's authorization is verified elsewhere, but without this the target
/// `user_id` is operated on globally, regardless of which organization it
/// belongs to. A target outside the path organization resolves to `NOT_FOUND`
/// so membership of other organizations is not disclosed.
pub(crate) struct OrganizationUserAccess(pub users::Model);

#[async_trait]
impl<S> FromRequestParts<S> for OrganizationUserAccess
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
        let user_id = parse_path_id(&path_params, "user_id")?;

        // Membership is the authorization: the target must hold a role in the
        // path organization. The same query yields the model the handler needs.
        let members =
            UserApi::find_by_organization(state.db_conn_ref(), organization_id)
                .await
                .map_err(|err| {
                    error!(
                        "find_by_organization({organization_id:?}) failed while verifying target user membership: {err:?}"
                    );
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to verify organization membership".to_string(),
                    )
                })?;

        members
            .into_iter()
            .find(|user| user.id == user_id)
            .map(OrganizationUserAccess)
            .ok_or((StatusCode::NOT_FOUND, "NOT FOUND".to_string()))
    }
}

fn parse_path_id(params: &HashMap<String, String>, key: &str) -> Result<Id, RejectionType> {
    params
        .get(key)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, format!("Missing {key} in path")))?
        .parse::<Id>()
        .map_err(|_| (StatusCode::BAD_REQUEST, format!("Invalid {key}")))
}
