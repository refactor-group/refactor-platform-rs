use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
use domain::{
    coaching_relationship as CoachingRelationshipApi, coaching_relationships,
    coaching_session_series as CoachingSessionSeriesApi, Id,
};

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, RejectionType},
    AppState,
};
use log::*;

/// Extracts a coaching session series and verifies the authenticated user is
/// a participant (coach OR coachee) of its parent relationship.
///
/// Used by read-only routes: `GET /coaching_session_series/:id`.
pub(crate) struct CoachingSessionSeriesAccess(pub CoachingSessionSeriesApi::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionSeriesAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let (series, relationship, user) = resolve(parts, state).await?;
        if relationship.grants_access_to(&user) {
            Ok(CoachingSessionSeriesAccess(series))
        } else {
            Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()))
        }
    }
}

/// Coach-only variant. Used by write routes: `PUT` (reschedule) and `DELETE`
/// on `/coaching_session_series/:id`.
pub(crate) struct CoachingSessionSeriesCoachAccess(pub CoachingSessionSeriesApi::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionSeriesCoachAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let (series, relationship, user) = resolve(parts, state).await?;
        if relationship.coach_id == user.id && relationship.grants_access_to(&user) {
            Ok(CoachingSessionSeriesCoachAccess(series))
        } else {
            Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()))
        }
    }
}

/// Shared lookup: extract `:id` from the path, load the series, load its
/// parent relationship, and pull the authenticated user. The caller decides
/// what membership rule to enforce.
async fn resolve<S>(
    parts: &mut Parts,
    state: &S,
) -> Result<
    (
        CoachingSessionSeriesApi::Model,
        coaching_relationships::Model,
        domain::users::Model,
    ),
    RejectionType,
>
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    let state = AppState::from_ref(state);
    let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, &state).await?;

    let Path(path_params) = Path::<HashMap<String, String>>::from_request_parts(parts, &state)
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid path parameters".to_string(),
            )
        })?;

    let series_id: Id = path_params
        .get("id")
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing series id in path".to_string(),
            )
        })?
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid series id".to_string()))?;

    debug!("Checking coaching_session_series access for series_id={series_id}");

    let series = CoachingSessionSeriesApi::find_by_id(state.db_conn_ref(), series_id)
        .await
        .map_err(|e| {
            error!("Error finding coaching_session_series {series_id}: {e:?}");
            (StatusCode::NOT_FOUND, "NOT FOUND".to_string())
        })?;

    let relationship =
        CoachingRelationshipApi::find_by_id(state.db_conn_ref(), series.coaching_relationship_id)
            .await
            .map_err(|e| {
                error!(
                    "Error finding coaching relationship {} for series {series_id}: {e:?}",
                    series.coaching_relationship_id
                );
                (StatusCode::NOT_FOUND, "NOT FOUND".to_string())
            })?;

    Ok((series, relationship, user))
}
