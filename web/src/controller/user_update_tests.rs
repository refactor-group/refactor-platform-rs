use crate::router::user_routes;
use crate::AppState;
use axum::http::StatusCode;
use axum::{body::Body, extract::Request, routing::post, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::Utc;
use domain::user::Backend;
use domain::{user_roles, users, Id};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, MockDatabase};
use service::config::Config;
use std::sync::Arc;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

fn requester() -> users::Model {
    let now = Utc::now();
    users::Model {
        id: Id::new_v4(),
        email: "test@example.com".to_string(),
        first_name: "Test".to_string(),
        last_name: "User".to_string(),
        display_name: None,
        password: Some(generate_hash("password123")),
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: domain::duration::Duration::default_minutes(),
        working_hours_start: domain::working_hours::default_start(),
        working_hours_end: domain::working_hours::default_end(),
        roles: vec![],
        invite_status: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// The production user routes behind a login, so the real guards run.
fn build_app(db: Arc<sea_orm::DatabaseConnection>) -> Router {
    let app_state = AppState::new(
        service::AppState::new(Config::default(), &db),
        Arc::new(sse::Manager::default()),
        domain::events::EventPublisher::default(),
        None,
        None,
        None,
    );

    let session_layer = SessionManagerLayer::new(MemoryStore::default())
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::days(1)))
        .with_always_save(true);
    let auth_layer = AuthManagerLayerBuilder::new(Backend::new(&db), session_layer).build();

    Router::new()
        .route(
            "/login",
            post(crate::controller::user_session_controller::login),
        )
        .with_state(app_state.clone())
        .merge(user_routes(app_state))
        .layer(auth_layer)
}

async fn login_cookie(app: &Router) -> String {
    let login_request = Request::builder()
        .uri("/login")
        .method("POST")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("email=test@example.com&password=password123"))
        .unwrap();
    let login_response = app.clone().oneshot(login_request).await.unwrap();
    login_response
        .headers()
        .get("set-cookie")
        .and_then(|c| c.to_str().ok())
        .expect("login should return a session cookie")
        .to_string()
}

#[tokio::test]
async fn put_own_user_with_an_off_step_start_returns_422() {
    let user = requester();
    let role = user_roles::Model {
        id: Id::new_v4(),
        role: users::Role::User,
        organization_id: Some(Id::new_v4()),
        user_id: user.id,
        created_at: Utc::now().into(),
        updated_at: Utc::now().into(),
    };

    // Login, then the session lookup on the PUT; nothing is queued for a write.
    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role)]])
            .into_connection(),
    );
    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let request = Request::builder()
        .uri(format!("/users/{}", user.id))
        .method("PUT")
        .header("cookie", cookie)
        .header("x-version", "1.0.0")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"working_hours_start":"08:15"}"#))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "status_code": 422,
            "error": "validation_error",
            "message": "working_hours_start must be HH:MM on a 30-minute step",
        })
    );
}
