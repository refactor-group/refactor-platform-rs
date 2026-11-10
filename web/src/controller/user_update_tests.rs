use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::Response;
use axum::{body::Body, extract::Request, routing::post, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::{NaiveTime, Utc};
use domain::user::Backend;
use domain::{user_roles, users, Id};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase};
use service::config::Config;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use crate::router::user_routes;
use crate::AppState;

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

fn role_for(user: &users::Model) -> user_roles::Model {
    user_roles::Model {
        id: Id::new_v4(),
        role: users::Role::User,
        organization_id: Some(Id::new_v4()),
        user_id: user.id,
        created_at: Utc::now().into(),
        updated_at: Utc::now().into(),
    }
}

/// Login and the PUT's session lookup, then whatever `then` stocks.
fn signed_in(
    user: &users::Model,
    then: impl FnOnce(MockDatabase) -> MockDatabase,
) -> Arc<DatabaseConnection> {
    let role = role_for(user);
    Arc::new(
        then(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role)]]),
        )
        .into_connection(),
    )
}

async fn put_user(app: Router, cookie: Option<String>, user_id: Id, body: &str) -> Response {
    let request = cookie
        .into_iter()
        .fold(Request::builder(), |request, cookie| {
            request.header("cookie", cookie)
        })
        .uri(format!("/users/{user_id}"))
        .method("PUT")
        .header("x-version", "1.0.0")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    app.oneshot(request).await.unwrap()
}

/// Every UPDATE the database ran, with its values inlined.
fn updates(db: &DatabaseConnection) -> Vec<String> {
    DatabaseConnection::clone(db)
        .into_transaction_log()
        .iter()
        .flat_map(|t| t.statements().iter().map(|s| s.to_string()))
        .filter(|sql| sql.trim_start().to_uppercase().starts_with("UPDATE"))
        .collect()
}

#[tokio::test]
async fn put_own_user_saves_valid_working_hours() {
    let user = requester();
    let saved = users::Model {
        working_hours_start: NaiveTime::from_hms_opt(9, 30, 0).expect("valid time"),
        working_hours_end: NaiveTime::from_hms_opt(18, 0, 0).expect("valid time"),
        ..user.clone()
    };
    let db = signed_in(&user, |db| {
        db.append_query_results([vec![(user.clone(), role_for(&user))]])
            .append_query_results([vec![saved]])
    });
    let app = build_app(db.clone());
    let cookie = login_cookie(&app).await;

    let response = put_user(
        app,
        Some(cookie),
        user.id,
        r#"{"working_hours_start":"09:30","working_hours_end":"18:00"}"#,
    )
    .await;

    // The `ApiResponse` envelope carries 204 in its body over an HTTP 200.
    assert_eq!(response.status(), StatusCode::OK);
    let updates = updates(&db);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(
        updates[0].contains("\"working_hours_start\" = '09:30:00"),
        "{}",
        updates[0]
    );
    assert!(
        updates[0].contains("\"working_hours_end\" = '18:00:00"),
        "{}",
        updates[0]
    );
}

#[tokio::test]
async fn put_another_users_working_hours_is_forbidden_and_writes_nothing() {
    let user = requester();
    let db = signed_in(&user, |db| db);
    let app = build_app(db.clone());
    let cookie = login_cookie(&app).await;

    let response = put_user(
        app,
        Some(cookie),
        Id::new_v4(),
        r#"{"working_hours_start":"09:30"}"#,
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(updates(&db).is_empty());
}

#[tokio::test]
async fn put_user_without_a_session_is_unauthorized_and_writes_nothing() {
    let user = requester();
    let db = signed_in(&user, |db| db);
    let app = build_app(db.clone());

    let response = put_user(app, None, user.id, r#"{"working_hours_start":"09:30"}"#).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(updates(&db).is_empty());
}
