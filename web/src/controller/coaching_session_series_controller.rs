use crate::controller::ApiResponse;
use crate::extractors::coaching_session_series_access::{
    CoachingRelationshipQueryAccess, CoachingSessionSeriesAccess, CoachingSessionSeriesCoachAccess,
};
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_session_series::{CreateParams, RescheduleParams};
use crate::{AppState, Error};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{
    coaching_session as CoachingSessionApi, coaching_session_series as CoachingSessionSeriesApi,
    emails as EmailsApi,
};
use serde::Serialize;
use service::config::ApiVersion;
use utoipa::ToSchema;

use log::*;

/// Response body for series-create and series-read endpoints.
#[derive(Debug, Serialize, ToSchema)]
pub struct SeriesWithSessions {
    pub series: CoachingSessionSeriesApi::Model,
    pub sessions: Vec<domain::coaching_sessions::Model>,
}

/// `POST /coaching_session_series` — create a recurring series and
/// materialize its sessions in one transaction.
#[utoipa::path(
    post,
    path = "/coaching_session_series",
    params(ApiVersion),
    request_body = CreateParams,
    responses(
        (status = 201, description = "Series created", body = SeriesWithSessions),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Invalid recurrence rule"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(("cookie_auth" = []))
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(params): Json<CreateParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST create coaching_session_series: {params:?}");

    let db = app_state.db_conn_ref();

    // Inline coach-only AuthZ — body-bearing routes parse the body in the
    // controller (matching the existing create_recurring pattern).
    let relationship =
        domain::coaching_relationship::find_by_id(db, params.coaching_relationship_id).await?;
    if relationship.coach_id != user.id {
        return Err(Error::Web(crate::error::WebErrorKind::Auth));
    }

    let requested_duration = CoachingSessionApi::parse_duration_minutes(params.duration_minutes)?;

    let (series, sessions) = CoachingSessionSeriesApi::create_with_sessions(
        db,
        params.coaching_relationship_id,
        relationship.coach_id,
        user.id,
        params.start_at,
        params.recurrence,
        requested_duration,
    )
    .await?;

    EmailsApi::notify_recurring_sessions_scheduled(db, &app_state.config, &sessions).await;

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        SeriesWithSessions { series, sessions },
    )))
}

/// `GET /coaching_session_series/:id` — read one series with its sessions.
#[utoipa::path(
    get,
    path = "/coaching_session_series/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Series ID")
    ),
    responses(
        (status = 200, description = "Series", body = SeriesWithSessions),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found")
    ),
    security(("cookie_auth" = []))
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    CoachingSessionSeriesAccess(series): CoachingSessionSeriesAccess,
) -> Result<impl IntoResponse, Error> {
    // The extractor already loaded the series row and validated access; do a
    // separate read for the linked sessions, ordered by date asc.
    let sessions =
        CoachingSessionApi::find_by_series_id(app_state.db_conn_ref(), series.id).await?;
    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        SeriesWithSessions { series, sessions },
    )))
}

/// `GET /coaching_session_series?coaching_relationship_id=...` — list series
/// metadata for a relationship (no nested sessions).
#[utoipa::path(
    get,
    path = "/coaching_session_series",
    params(
        ApiVersion,
        ("coaching_relationship_id" = Id, Query, description = "Filter by coaching relationship")
    ),
    responses(
        (status = 200, description = "Series list", body = [CoachingSessionSeriesApi::Model]),
        (status = 401, description = "Unauthorized")
    ),
    security(("cookie_auth" = []))
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    CoachingRelationshipQueryAccess(relationship): CoachingRelationshipQueryAccess,
) -> Result<impl IntoResponse, Error> {
    let series =
        CoachingSessionSeriesApi::find_by_relationship(app_state.db_conn_ref(), relationship.id)
            .await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), series)))
}

/// `PUT /coaching_session_series/:id` — reschedule. Replaces the rule and
/// re-materializes future sessions; past sessions stay put.
#[utoipa::path(
    put,
    path = "/coaching_session_series/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Series ID")
    ),
    request_body = RescheduleParams,
    responses(
        (status = 200, description = "Rescheduled", body = SeriesWithSessions),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Invalid recurrence rule")
    ),
    security(("cookie_auth" = []))
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    CoachingSessionSeriesCoachAccess(series): CoachingSessionSeriesCoachAccess,
    Json(params): Json<RescheduleParams>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "PUT reschedule coaching_session_series id={} params={params:?}",
        series.id
    );

    let db = app_state.db_conn_ref();
    let relationship =
        domain::coaching_relationship::find_by_id(db, series.coaching_relationship_id).await?;
    let requested_duration = CoachingSessionApi::parse_duration_minutes(params.duration_minutes)?;

    let (updated_series, new_sessions) = CoachingSessionSeriesApi::reschedule(
        db,
        &app_state.config,
        series.id,
        relationship.coach_id,
        params.start_at,
        params.recurrence,
        requested_duration,
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        SeriesWithSessions {
            series: updated_series,
            sessions: new_sessions,
        },
    )))
}

/// `DELETE /coaching_session_series/:id` — delete series + future sessions.
/// Past sessions survive as orphan one-offs (FK SET NULL).
#[utoipa::path(
    delete,
    path = "/coaching_session_series/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Series ID")
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized")
    ),
    security(("cookie_auth" = []))
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    CoachingSessionSeriesCoachAccess(series): CoachingSessionSeriesCoachAccess,
) -> Result<impl IntoResponse, Error> {
    CoachingSessionSeriesApi::delete_with_future_sessions(
        app_state.db_conn_ref(),
        &app_state.config,
        series.id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use chrono::Utc;
    use domain::{
        coaching_relationships,
        coaching_session::{Frequency, Recurrence},
        users, Id,
    };
    use sea_orm::{DatabaseBackend, MockDatabase};
    use service::config::Config;
    use std::sync::Arc;

    fn test_user(id: Id) -> users::Model {
        let now = Utc::now();
        users::Model {
            id,
            email: "user@example.com".to_string(),
            first_name: "Test".to_string(),
            last_name: "User".to_string(),
            display_name: None,
            password: None,
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            default_coaching_session_duration_minutes: domain::duration::Duration::default_minutes(
            ),
            role: users::Role::User,
            roles: vec![],
            invite_status: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_app_state(db: Arc<sea_orm::DatabaseConnection>) -> AppState {
        AppState::new(
            service::AppState::new(Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default(),
            None,
            None,
        )
    }

    #[tokio::test]
    async fn create_rejects_non_coach_with_auth_error() {
        let user = test_user(Id::new_v4());
        let now = Utc::now();
        let relationship = coaching_relationships::Model {
            id: Id::new_v4(),
            organization_id: Id::new_v4(),
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
            slug: "test".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        };
        assert_ne!(user.id, relationship.coach_id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![relationship.clone()]])
                .into_connection(),
        );
        let app_state = test_app_state(db);

        let params = CreateParams {
            coaching_relationship_id: relationship.id,
            start_at: chrono::NaiveDate::from_ymd_opt(2026, 6, 1)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
            recurrence: Recurrence {
                frequency: Frequency::Weekly,
                interval: 1,
                by_weekdays: None,
                count: Some(3),
                until: None,
            },
            duration_minutes: None,
        };

        let result = create(
            CompareApiVersion(HeaderValue::from_static("1.0.0")),
            AuthenticatedUser(user),
            State(app_state),
            Json(params),
        )
        .await;

        let err = result
            .err()
            .expect("expected the handler to reject a non-coach caller");
        assert!(
            matches!(err, Error::Web(crate::error::WebErrorKind::Auth)),
            "expected Err(Web(Auth)), got {err:?}"
        );
    }
}
