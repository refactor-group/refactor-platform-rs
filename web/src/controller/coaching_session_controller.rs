use crate::controller::ApiResponse;
use crate::extractors::coaching_session_access::CoachingSessionAccess;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_session::{
    CreateParams, IndexParams, SortField, TitleUpdateParams, UpdateParams,
};
use crate::params::WithSortDefaults;
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{coaching_session as CoachingSessionApi, emails as EmailsApi, Id};
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
        app_state.event_publisher.as_ref(),
        coaching_session,
    )
    .await?;
    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        coaching_session,
    )))
}

/// Mark a coaching session viewed by the caller, returning the prior marker.
///
/// Upserts the authenticated caller's view marker for this session to now() and returns the
/// value it had immediately before, so the caller can compute what is new since their last view.
/// Idempotent. Participant-only (same access as reading the session).
#[utoipa::path(
    post,
    path = "/coaching_sessions/{coaching_session_id}/view",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    responses(
        (status = 200, description = "Marker advanced; prior value returned", body = domain::coaching_session_view::MarkViewed),
        (status = 401, description = "Unauthorized or not a participant"),
        (status = 404, description = "Coaching session not found"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn view(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let result =
        domain::coaching_session_view::mark_viewed(app_state.db_conn_ref(), session.id, user.id)
            .await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), result)))
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
        (status = 200, description = "Successfully retrieved all Coaching Sessions", body = [domain::coaching_session::SessionWithDisplayTitle]),
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

    let coaching_sessions =
        CoachingSessionApi::find_by_with_display_title(app_state.db_conn_ref(), params).await?;

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

    // Validate wire `Option<i16>` → `Option<Duration>` so lower layers see
    // only already-validated values. Out-of-range propagates as 422.
    let requested_duration = CoachingSessionApi::parse_duration_minutes(params.duration_minutes)?;
    let coaching_session_model = params.into_model();

    let coaching_session = CoachingSessionApi::create(
        app_state.db_conn_ref(),
        &app_state.config,
        app_state.event_publisher.as_ref(),
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

/// PATCH update only the title of a Coaching Session.
///
/// Either participant (coach or coachee) may edit the title; the scheduling fields stay on the
/// coach-only `PUT /coaching_sessions/{id}`. Authorization is the `CoachingSessionAccess`
/// extractor (participant-gated). Returns the updated session.
#[utoipa::path(
    patch,
    path = "/coaching_sessions/{id}/title",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching Session ID to Update")
    ),
    request_body = TitleUpdateParams,
    responses(
        (status = 200, description = "Successfully updated the title", body = coaching_sessions::Model),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Title exceeds the maximum length"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update_title(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(coaching_session): CoachingSessionAccess,
    State(app_state): State<AppState>,
    Json(params): Json<TitleUpdateParams>,
) -> Result<impl IntoResponse, Error> {
    let updated =
        CoachingSessionApi::update(app_state.db_conn_ref(), coaching_session.id, params).await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated)))
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
    use sea_orm::{DatabaseBackend, MockDatabase};
    use service::config::Config;
    use std::sync::Arc;

    fn test_app_state(db: Arc<sea_orm::DatabaseConnection>) -> AppState {
        AppState::new(
            service::AppState::new(Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default(),
            None,
            None,
        )
    }

    fn test_session(id: Id, relationship_id: Id) -> domain::coaching_sessions::Model {
        let now = Utc::now();
        domain::coaching_sessions::Model {
            id,
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: None,
            collab_document_name: None,
            date: now.naive_utc(),
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
        }
    }

    // The participant gate lives in CoachingSessionAccess (tested there); here we assert the
    // handler wires a participant-authorized request through to a title update and returns the
    // updated session. Constructing the extractor directly stands in for a passed gate.
    #[tokio::test]
    async fn update_title_updates_and_returns_session() {
        let session = test_session(Id::new_v4(), Id::new_v4());
        let updated = domain::coaching_sessions::Model {
            title: Some("New title".to_string()),
            ..session.clone()
        };

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![session.clone()]]) // domain update: find_by_id
                .append_query_results(vec![vec![updated.clone()]]) // UPDATE ... RETURNING
                .into_connection(),
        );

        let result = update_title(
            CompareApiVersion(HeaderValue::from_static("1.0.0")),
            CoachingSessionAccess(session.clone()),
            State(test_app_state(db)),
            Json(TitleUpdateParams {
                title: Some(Some("New title".to_string())),
            }),
        )
        .await;

        result.expect("participant title update should succeed");
    }

    // Title-only map: a value sets it, explicit null clears it, absence leaves it untouched.
    #[tokio::test]
    async fn title_update_params_build_a_title_only_map() {
        use domain::IntoUpdateMap;
        use sea_orm::Value;

        let set = TitleUpdateParams {
            title: Some(Some("Hi".to_string())),
        }
        .into_update_map();
        assert!(matches!(
            set.get_value("title"),
            Some(Value::String(Some(_)))
        ));

        let clear = TitleUpdateParams { title: Some(None) }.into_update_map();
        assert!(matches!(
            clear.get_value("title"),
            Some(Value::String(None))
        ));

        let absent = TitleUpdateParams { title: None }.into_update_map();
        assert!(
            absent.get_value("title").is_none(),
            "absent title is a no-op"
        );
    }
}
