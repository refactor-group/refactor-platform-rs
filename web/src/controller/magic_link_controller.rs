use crate::{controller::ApiResponse, params::user::CompleteSetupParams, AppState, Error};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::magic_link_token::{self as MagicLinkTokenApi};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct ValidateParams {
    pub token: String,
}

/// GET /magic-link/validate
///
/// Validate a magic link token without consuming it.
///
/// Returns the user's profile data so the frontend can pre-fill the setup form.

#[utoipa::path(
    get,
    path = "/magic-link/validate",
    params(
        ("token" = String, Query, description = "Magic login token from the welcome email"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved a User", body = User),
        (status = 400, description = "Invalid login token"),
        (status = 401, description = "Expired token"),
    )
)]
pub(crate) async fn validate(
    State(app_state): State<AppState>,
    Query(params): Query<ValidateParams>,
) -> Result<impl IntoResponse, Error> {
    let user = MagicLinkTokenApi::validate_token(app_state.db_conn_ref(), &params.token).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), user)))
}

/// POST /magic-link/complete-setup
///
/// Consume a magic link token and complete user account setup.
///
/// Sets the user's password and optionally updates profile fields.
/// The token is deleted after successful consumption.
///
#[utoipa::path(
    post,
    path = "/magic-link/complete-setup",
    request_body = CompleteSetupParams,
    responses(
        (status = 200, description = "User profile successfully updated", body = User),
        (status = 422, description = "Password confirmation does not match"),
        (status = 503, description = "Service temporarily unavailable")
    )
)]
pub(crate) async fn complete_setup(
    State(app_state): State<AppState>,
    Json(params): Json<CompleteSetupParams>,
) -> Result<impl IntoResponse, Error> {
    let updated_user = MagicLinkTokenApi::complete_setup(app_state.db_conn_ref(), params).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated_user)))
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        response::Response,
        routing::post,
        Router,
    };
    use axum_login::{
        tower_sessions::{Expiry, MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use chrono::{Duration, Utc};
    use domain::events::EventPublisher;
    use domain::user::Backend;
    use domain::{magic_link_tokens, user_roles, users, Id};
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};
    use service::config::Config;
    use std::sync::Arc;
    use time::Duration as TimeDuration;
    use tower::ServiceExt;

    fn test_token_model(user_id: Id) -> magic_link_tokens::Model {
        magic_link_tokens::Model {
            id: Id::new_v4(),
            user_id,
            token_hash: "mocked_hash".to_string(),
            expires_at: (Utc::now() + Duration::hours(24)).into(),
            created_at: Utc::now().into(),
        }
    }

    fn test_user_model(id: Id) -> users::Model {
        users::Model {
            id,
            email: "invitee@example.com".to_string(),
            first_name: "New".to_string(),
            last_name: "User".to_string(),
            display_name: None,
            password: None,
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            role: users::Role::User,
            roles: vec![],
            invite_status: None,
            created_at: Utc::now().into(),
            updated_at: Utc::now().into(),
        }
    }

    fn build_app(db: Arc<sea_orm::DatabaseConnection>) -> Router {
        let config = Config::default();
        let service_state = service::AppState::new(config, &db);
        let sse_manager = Arc::new(sse::Manager::new());
        let event_publisher = EventPublisher::new();
        let app_state = crate::AppState::new(service_state, sse_manager, event_publisher);

        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(TimeDuration::days(1)));

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        Router::new()
            .route("/magic-link/complete-setup", post(super::complete_setup))
            .layer(auth_layer)
            .with_state(app_state)
    }

    #[tokio::test]
    async fn complete_setup_returns_200_for_valid_token() {
        let user_id = Id::new_v4();
        let token_model = test_token_model(user_id);
        let user = test_user_model(user_id);
        let updated_user = users::Model {
            password: Some("hashed_new_password".into()),
            ..user.clone()
        };

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                // find_by_token_hash
                .append_query_results(vec![vec![token_model]])
                // find_by_id (uses find_with_related)
                .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>(vec![
                    vec![(user, None)],
                ])
                // delete_all_for_user
                .append_exec_results(vec![MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                }])
                // mutate::update
                .append_query_results(vec![vec![updated_user]])
                .into_connection(),
        );

        let app = build_app(db);

        let request = Request::builder()
            .uri("/magic-link/complete-setup")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "token": "valid_raw_token",
                    "password": "SecurePassword123!",
                    "confirm_password": "SecurePassword123!"
                }))
                .unwrap(),
            ))
            .unwrap();

        let response: Response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn complete_setup_returns_401_for_expired_token() {
        let user_id = Id::new_v4();
        let expired_token = magic_link_tokens::Model {
            expires_at: (Utc::now() - Duration::hours(1)).into(),
            ..test_token_model(user_id)
        };

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                // find_by_token_hash returns the expired token
                .append_query_results(vec![vec![expired_token]])
                .into_connection(),
        );

        let app = build_app(db);

        let request = Request::builder()
            .uri("/magic-link/complete-setup")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "token": "expired_raw_token",
                    "password": "SecurePassword123!",
                    "confirm_password": "SecurePassword123!"
                }))
                .unwrap(),
            ))
            .unwrap();

        let response: Response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn complete_setup_returns_404_for_invalid_token() {
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                // find_by_token_hash returns None (token does not exist)
                .append_query_results(vec![Vec::<magic_link_tokens::Model>::new()])
                .into_connection(),
        );

        let app = build_app(db);

        let request = Request::builder()
            .uri("/magic-link/complete-setup")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "token": "nonexistent_token",
                    "password": "SecurePassword123!",
                    "confirm_password": "SecurePassword123!"
                }))
                .unwrap(),
            ))
            .unwrap();

        let response: Response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn complete_setup_returns_422_for_mismatched_passwords() {
        let db = Arc::new(MockDatabase::new(DatabaseBackend::Postgres).into_connection());

        let app = build_app(db);

        let request = Request::builder()
            .uri("/magic-link/complete-setup")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({
                    "token": "any_token",
                    "password": "password123",
                    "confirm_password": "different456"
                }))
                .unwrap(),
            ))
            .unwrap();

        let response: Response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            json["status_code"],
            StatusCode::UNPROCESSABLE_ENTITY.as_u16()
        );
        assert_eq!(json["error"], "validation_error");
        assert_eq!(json["message"], "Password confirmation does not match");
    }
}
