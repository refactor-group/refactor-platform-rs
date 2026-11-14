use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
use domain::{coaching_session_series as CoachingSessionSeriesApi, Id};

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser,
        coaching_relationship_access::{authorize_relationship, authorize_relationship_coach},
        lookup_rejection, RejectionType,
    },
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
        let (state, series, user) = resolve(parts, state).await?;
        authorize_relationship(&state, &user, series.coaching_relationship_id).await?;
        Ok(CoachingSessionSeriesAccess(series))
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
        let (state, series, user) = resolve(parts, state).await?;
        authorize_relationship_coach(&state, &user, series.coaching_relationship_id).await?;
        Ok(CoachingSessionSeriesCoachAccess(series))
    }
}

/// Shared lookup: pull the authenticated user, extract `:id` from the path, and load the
/// series. The caller decides what membership rule to enforce on its relationship.
async fn resolve<S>(
    parts: &mut Parts,
    state: &S,
) -> Result<
    (
        AppState,
        CoachingSessionSeriesApi::Model,
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
        .map_err(|e| lookup_rejection(e.into()))?;

    Ok((state, series, user))
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use axum::{response::Response, routing::get, Router};
    use chrono::Utc;
    use sea_orm::{DbErr, MockDatabase};

    use super::*;
    use crate::extractors::relationship_access_test_support::{self as support, Caller};

    async fn read_series(_access: CoachingSessionSeriesAccess) -> &'static str {
        "extracted_success"
    }

    async fn change_series(_access: CoachingSessionSeriesCoachAccess) -> &'static str {
        "extracted_success"
    }

    fn series(w: &support::Fixture) -> CoachingSessionSeriesApi::Model {
        let now = Utc::now();
        CoachingSessionSeriesApi::Model {
            id: Id::new_v4(),
            coaching_relationship_id: w.relationship.id,
            rule: serde_json::json!({}),
            ical_sequence: 0,
            created_by_user_id: w.coach.id,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    /// Signs the coach in over `db`, then sends `method` to the series.
    async fn coach_sends(
        db: MockDatabase,
        w: &support::Fixture,
        method: &str,
        series_id: Id,
    ) -> Response {
        let db = Arc::new(db.into_connection());
        let routes = Router::new().route(
            "/coaching_session_series/:id",
            get(read_series).delete(change_series),
        );
        support::send(
            routes,
            &db,
            &w.coach.email,
            method,
            &format!("/coaching_session_series/{series_id}"),
            None,
        )
        .await
    }

    #[tokio::test]
    async fn a_failed_series_lookup_is_a_server_error() {
        let w = support::fixture();
        let db = w
            .signed_in(Caller::Coach)
            .append_query_errors([DbErr::Custom("connection lost".to_string())]);

        let response = coach_sends(db, &w, "GET", Id::new_v4()).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn a_failed_relationship_lookup_after_a_found_series_is_a_server_error() {
        let w = support::fixture();
        let series = series(&w);
        let series_id = series.id;
        let db = w
            .signed_in(Caller::Coach)
            .append_query_results([vec![series]])
            .append_query_errors([DbErr::Custom("connection lost".to_string())]);

        let response = coach_sends(db, &w, "DELETE", series_id).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
