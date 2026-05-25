use crate::controller::ApiResponse;
use crate::error::WebErrorKind;
use crate::extractors::coaching_session_access::CoachingSessionAccess;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_session::recurring::CreateRecurringParams;
use crate::params::coaching_session::{CreateParams, IndexParams, SortField, UpdateParams};
use crate::params::WithSortDefaults;
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{
    coaching_relationship as CoachingRelationshipApi, coaching_session as CoachingSessionApi,
    duration::Duration, emails as EmailsApi, Id,
};
use service::config::ApiVersion;

use log::*;

/// GET a Coaching Session by ID
#[utoipa::path(
    get,
    path = "/coaching_sessions/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching Session ID to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a Coaching Session", body = coaching_sessions::Model),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Coaching Session not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    CoachingSessionAccess(coaching_session): CoachingSessionAccess,
) -> Result<impl IntoResponse, Error> {
    let coaching_session = CoachingSessionApi::ensure_hydrated(
        app_state.db_conn_ref(),
        &app_state.config,
        coaching_session,
    )
    .await?;
    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        coaching_session,
    )))
}

#[utoipa::path(
    get,
    path = "/coaching_sessions",
    params(
        ApiVersion,
        ("coaching_relationship_id" = Option<Id>, Query, description = "Filter by coaching_relationship_id"),
        ("from_date" = Option<NaiveDate>, Query, description = "Filter by from_date"),
        ("to_date" = Option<NaiveDate>, Query, description = "Filter by to_date"),
        ("sort_by" = Option<crate::params::coaching_session::SortField>, Query, description = "Sort by field. Valid values: 'date', 'created_at', 'updated_at'. Must be provided with sort_order.", example = "date"),
        ("sort_order" = Option<crate::params::sort::SortOrder>, Query, description = "Sort order. Valid values: 'asc' (ascending), 'desc' (descending). Must be provided with sort_by.", example = "desc")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Coaching Sessions", body = [coaching_sessions::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET all Coaching Sessions");
    debug!("Filter Params: {params:?}");

    // Apply default sorting parameters
    let mut params = params;
    IndexParams::apply_sort_defaults(&mut params.sort_by, &mut params.sort_order, SortField::Date);

    let coaching_sessions = CoachingSessionApi::find_by(app_state.db_conn_ref(), params).await?;

    debug!("Found Coaching Sessions: {coaching_sessions:?}");

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        coaching_sessions,
    )))
}

/// POST create a new Coaching Session
#[utoipa::path(
    post,
    path = "/coaching_sessions",
    params(ApiVersion),
    request_body = CreateParams,
    responses(
        (status = 201, description = "Successfully Created a new Coaching Session", body = [domain::coaching_sessions::Model]),
        (status= 422, description = "Unprocessable Entity"),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Json(params): Json<CreateParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST Create a new Coaching Session from: {params:?}");

    // Wire input is `Option<u16>` — validate at the controller boundary so the
    // domain and entity_api layers see only `Option<Duration>` (already valid
    // by the type). Out-of-range values propagate as 422.
    let requested_duration: Option<Duration> = params
        .duration_minutes
        .map(Duration::try_from)
        .transpose()?;
    let coaching_session_model = params.into_model();

    let coaching_session = CoachingSessionApi::create(
        app_state.db_conn_ref(),
        &app_state.config,
        coaching_session_model,
        requested_duration,
    )
    .await?;

    debug!("New Coaching Session: {coaching_session:?}");

    EmailsApi::notify_session_scheduled(
        app_state.db_conn_ref(),
        &app_state.config,
        &coaching_session,
    )
    .await;

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        coaching_session,
    )))
}

/// POST create a recurring series of coaching sessions in one request.
/// Returns the inserted rows.
///
/// Each session's meeting provider (Zoom, Google Meet, etc.) is resolved
/// lazily on first read using the coach's then-current OAuth connection —
/// not at the time this endpoint is called. If the coach reconnects a
/// different provider before opening a session, that session will use the
/// new provider.
#[utoipa::path(
    post,
    path = "/coaching_sessions/recurring",
    params(ApiVersion),
    request_body = CreateRecurringParams,
    responses(
        (status = 201, description = "Successfully created the recurring series", body = [domain::coaching_sessions::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 422, description = "Unprocessable Entity (invalid recurrence rule)"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn create_recurring(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(params): Json<CreateRecurringParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST Create recurring coaching sessions: {params:?}");

    let db = app_state.db_conn_ref();

    let dates = CoachingSessionApi::expand_recurrence(params.start_at, &params.recurrence)?;

    let relationship =
        CoachingRelationshipApi::find_by_id(db, params.coaching_relationship_id).await?;
    if relationship.coach_id != user.id {
        return Err(Error::Web(WebErrorKind::Auth));
    }

    // Validate duration at the wire boundary (see `create` above).
    let requested_duration: Option<Duration> = params
        .duration_minutes
        .map(Duration::try_from)
        .transpose()?;

    let sessions = CoachingSessionApi::bulk_create_recurring(
        db,
        params.coaching_relationship_id,
        relationship.coach_id,
        dates,
        requested_duration,
    )
    .await?;

    EmailsApi::notify_recurring_sessions_scheduled(db, &app_state.config, &sessions).await;

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), sessions)))
}

/// PUT update a Coaching Session
#[utoipa::path(
    put,
    path = "/coaching_sessions/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching Session ID to Update")
    ),
    request_body = UpdateParams,
    responses(
        (status = 204, description = "Successfully updated a Coaching Session", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(coaching_session_id): Path<Id>,
    Json(params): Json<UpdateParams>,
) -> Result<impl IntoResponse, Error> {
    CoachingSessionApi::update(app_state.db_conn_ref(), coaching_session_id, params).await?;
    Ok(Json(ApiResponse::new(StatusCode::NO_CONTENT.into(), ())))
}

/// DELETE a Coaching Session
#[utoipa::path(
    delete,
    path = "/coaching_sessions/{id}",
    params(ApiVersion, ("id" = Id, Path, description = "Coaching Session ID to Delete")),
    responses(
        (status = 204, description = "Successfully deleted a Coaching Session", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(coaching_session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    CoachingSessionApi::delete(
        app_state.db_conn_ref(),
        &app_state.config,
        coaching_session_id,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::NO_CONTENT.into(), ())))
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
        users,
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
    async fn create_recurring_rejects_non_coach_with_auth_error() {
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

        let params = CreateRecurringParams {
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

        let result = create_recurring(
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
            matches!(err, Error::Web(WebErrorKind::Auth)),
            "expected Err(Web(Auth)), got {err:?}"
        );
    }
}
