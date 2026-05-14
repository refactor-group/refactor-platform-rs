use crate::controller::ApiResponse;
use crate::error::WebErrorKind;
use crate::extractors::{
    coaching_session_access::CoachingSessionAccess, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::meeting_recording as MeetingRecordingApi;
use domain::meeting_recording::MeetingRecordingStatus;
use log::*;
use serde::Deserialize;
use service::config::ApiVersion;

#[derive(Debug, Deserialize)]
pub struct StartRecordingParams {
    pub meeting_url: String,
}

/// GET the current recording status and artifact URLs for a coaching session
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/meeting_recording",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    responses(
        (status = 200, description = "Recording status retrieved"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let coaching_session_id = session.id;
    debug!("GET meeting_recording for session {}", coaching_session_id);

    let recording = MeetingRecordingApi::find_latest_by_coaching_session(
        app_state.db_conn_ref(),
        coaching_session_id,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), recording)))
}

/// POST create a Recall.ai bot and start recording a coaching session
#[utoipa::path(
    post,
    path = "/coaching_sessions/{coaching_session_id}/meeting_recording",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    request_body = StartRecordingParams,
    responses(
        (status = 201, description = "Recording bot created and joined meeting"),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "An active recording already exists for this session"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
    Json(params): Json<StartRecordingParams>,
) -> Result<impl IntoResponse, Error> {
    let coaching_session_id = session.id;
    debug!("POST meeting_recording for session {}", coaching_session_id);

    if params.meeting_url.trim().is_empty() {
        return Err(Error::Web(WebErrorKind::Input));
    }

    // Prevent duplicate active bots
    if let Some(existing) = MeetingRecordingApi::find_latest_by_coaching_session(
        app_state.db_conn_ref(),
        coaching_session_id,
    )
    .await?
    {
        let active = !matches!(
            existing.status,
            MeetingRecordingStatus::Failed
                | MeetingRecordingStatus::Completed
                | MeetingRecordingStatus::Cancelled
        );
        if active {
            warn!(
                "Active recording {} already exists for session {}",
                existing.id, coaching_session_id
            );
            return Err(Error::Web(WebErrorKind::Conflict));
        }
    }

    let recording = MeetingRecordingApi::start(
        app_state.db_conn_ref(),
        app_state.recording_bot_provider.as_deref(),
        coaching_session_id,
        &params.meeting_url,
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        recording,
    )))
}

/// DELETE stop the active recording bot for a coaching session
#[utoipa::path(
    delete,
    path = "/coaching_sessions/{coaching_session_id}/meeting_recording",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    responses(
        (status = 200, description = "Recording bot stopped"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No active recording found for this session"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let coaching_session_id = session.id;
    debug!(
        "DELETE meeting_recording for session {}",
        coaching_session_id
    );

    let recording = MeetingRecordingApi::stop(
        app_state.db_conn_ref(),
        app_state.recording_bot_provider.as_deref(),
        coaching_session_id,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), recording)))
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use axum::{body::Body, extract::Request, middleware::from_fn, routing::post, Router};
    use axum_login::{
        tower_sessions::{MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use chrono::Utc;
    use domain::meeting_recording::Model as RecordingModel;
    use domain::user::Backend;
    use domain::{coaching_relationships, coaching_sessions, user_roles, users, Id};
    use password_auth::generate_hash;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use service::config::Config;
    use std::sync::Arc;
    use time::Duration;
    use tower::ServiceExt;
    use tower_sessions::Expiry;

    const X_VERSION: &str = "x-version";
    const API_VERSION: &str = "1.0.0-beta1";

    fn test_user() -> users::Model {
        let now = Utc::now();
        users::Model {
            id: Id::new_v4(),
            email: "coach@example.com".to_string(),
            first_name: "Coach".to_string(),
            last_name: "User".to_string(),
            display_name: None,
            password: Some(generate_hash("password123".to_string())),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            role: users::Role::User,
            roles: vec![],
            invite_status: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_role(user_id: Id) -> user_roles::Model {
        let now = Utc::now();
        user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(Id::new_v4()),
            user_id,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_recording(session_id: Id, status: MeetingRecordingStatus) -> RecordingModel {
        let now = Utc::now();
        RecordingModel {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            bot_id: "bot-existing".to_string(),
            status,
            video_url: None,
            audio_url: None,
            duration_seconds: None,
            started_at: None,
            ended_at: None,
            error_message: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn build_app(db: Arc<sea_orm::DatabaseConnection>) -> Router {
        let config = Config::default();
        let service_state = service::AppState::new(config, &db);
        let app_state = AppState::new(
            service_state,
            Arc::new(sse::Manager::new()),
            domain::events::EventPublisher::default(),
            None,
            None,
        );

        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::days(1)))
            .with_always_save(true);

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        Router::new()
            .route(
                "/login",
                axum::routing::post(crate::controller::user_session_controller::login),
            )
            .merge(
                Router::new()
                    .route(
                        "/coaching_sessions/:coaching_session_id/meeting_recording",
                        post(create),
                    )
                    .route_layer(from_fn(crate::middleware::auth::require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state)
    }

    async fn do_login(app: &Router, email: &str, password: &str) -> String {
        let req = Request::builder()
            .uri("/login")
            .method("POST")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(format!("email={email}&password={password}")))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        resp.headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
            .expect("login must return session cookie")
            .to_string()
    }

    #[tokio::test]
    async fn create_returns_409_when_active_recording_exists() {
        let user = test_user();
        let role = test_role(user.id);
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let now = Utc::now();

        let session = coaching_sessions::Model {
            id: session_id,
            coaching_relationship_id: relationship_id,
            collab_document_name: None,
            date: now.naive_utc(),
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let relationship = coaching_relationships::Model {
            id: relationship_id,
            coach_id: Id::new_v4(),
            coachee_id: user.id,
            organization_id: Id::new_v4(),
            slug: "test".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let active_recording = test_recording(session_id, MeetingRecordingStatus::Recording);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![(session, relationship)]])
                .append_query_results(vec![vec![active_recording]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app, "coach@example.com", "password123").await;

        let req = Request::builder()
            .uri(format!("/coaching_sessions/{session_id}/meeting_recording"))
            .method("POST")
            .header("cookie", cookie)
            .header("content-type", "application/json")
            .header(X_VERSION, API_VERSION)
            .body(Body::from(r#"{"meeting_url":"https://zoom.us/j/123456"}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn create_returns_400_for_empty_meeting_url() {
        let user = test_user();
        let role = test_role(user.id);
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let now = Utc::now();

        let session = coaching_sessions::Model {
            id: session_id,
            coaching_relationship_id: relationship_id,
            collab_document_name: None,
            date: now.naive_utc(),
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let relationship = coaching_relationships::Model {
            id: relationship_id,
            coach_id: Id::new_v4(),
            coachee_id: user.id,
            organization_id: Id::new_v4(),
            slug: "test".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![(session, relationship)]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app, "coach@example.com", "password123").await;

        let req = Request::builder()
            .uri(format!("/coaching_sessions/{session_id}/meeting_recording"))
            .method("POST")
            .header("cookie", cookie)
            .header("content-type", "application/json")
            .header(X_VERSION, API_VERSION)
            .body(Body::from(r#"{"meeting_url":"   "}"#))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
