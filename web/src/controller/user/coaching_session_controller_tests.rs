use crate::controller::user::coaching_session_controller::nearest;
use crate::middleware::auth::require_auth;
use crate::{protect, AppState};
use axum::http::StatusCode;
use axum::middleware::{from_fn, from_fn_with_state};
use axum::{body::Body, extract::Request, routing::get, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::{NaiveDate, Utc};
use domain::user::Backend;
use domain::{coaching_relationships, coaching_sessions, user_roles, users, Id};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, MockDatabase};
use serde_json::{json, Value};
use service::config::Config;
use std::sync::Arc;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

fn test_user() -> users::Model {
    let now = Utc::now();
    users::Model {
        id: Id::new_v4(),
        email: "test@example.com".to_string(),
        first_name: "Test".to_string(),
        last_name: "User".to_string(),
        display_name: Some("Test User".to_string()),
        password: Some(generate_hash("password123")),
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: domain::duration::Duration::default_minutes(),
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

fn relationship(id: Id, coach_id: Id) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        id,
        organization_id: Id::new_v4(),
        coach_id,
        coachee_id: Id::new_v4(),
        slug: "coach-coachee".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn session(coaching_relationship_id: Id) -> coaching_sessions::Model {
    let now = Utc::now();
    coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: NaiveDate::from_ymd_opt(2026, 10, 29)
            .unwrap()
            .and_hms_opt(14, 0, 0)
            .unwrap(),
        duration_minutes: domain::duration::Duration::default_minutes(),
        title: Some("Quarterly review".to_string()),
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: None,
        notice_given_at: now.into(),
    }
}

/// The login's two user-and-role reads.
fn logged_in(user: &users::Model) -> MockDatabase {
    let row = vec![(user.clone(), test_role(user.id))];
    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([row.clone()])
        .append_query_results([row])
}

fn build_app(db: Arc<sea_orm::DatabaseConnection>) -> Router {
    let state = AppState::new(
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
            axum::routing::post(crate::controller::user_session_controller::login),
        )
        .merge(
            Router::new()
                .route("/users/:user_id/coaching_sessions/nearest", get(nearest))
                .route_layer(from_fn_with_state(
                    state.clone(),
                    protect::users::coaching_sessions::index,
                ))
                .route_layer(from_fn(require_auth)),
        )
        .layer(auth_layer)
        .with_state(state)
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

async fn get_nearest(app: &Router, user_id: Id, query: &str) -> axum::response::Response {
    let cookie = login_cookie(app).await;
    let request = Request::builder()
        .uri(format!(
            "/users/{user_id}/coaching_sessions/nearest?{query}"
        ))
        .header("cookie", cookie)
        .header("x-version", "1.0.0")
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).expect("the body is JSON")
}

fn window_query(relationship_id: Id) -> String {
    format!(
        "coaching_relationship_id={relationship_id}&around=2026-10-30T14:00:00Z&before=2&after=3"
    )
}

#[tokio::test]
async fn an_invalid_window_is_a_400_with_the_invalid_nearest_window_slug() {
    let user = test_user();
    let app = build_app(Arc::new(logged_in(&user).into_connection()));

    let response = get_nearest(&app, user.id, "around=2026-10-30T14:00:00&before=2&after=3").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json_body(response).await,
        json!({
            "status_code": 400,
            "error": "invalid_nearest_window",
            "message": "`coaching_relationship_id` is required.",
        })
    );
}

#[tokio::test]
async fn another_users_sessions_are_forbidden() {
    let user = test_user();
    let app = build_app(Arc::new(logged_in(&user).into_connection()));

    let response = get_nearest(&app, Id::new_v4(), &window_query(Id::new_v4())).await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_relationship_the_user_is_not_in_is_an_empty_200() {
    let user = test_user();
    let db = logged_in(&user)
        .append_query_results::<coaching_relationships::Model, Vec<_>, _>(vec![vec![]])
        .into_connection();
    let app = build_app(Arc::new(db));

    let response = get_nearest(&app, user.id, &window_query(Id::new_v4())).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await,
        json!({ "status_code": 200, "data": [] })
    );
}

#[tokio::test]
async fn the_windows_sessions_come_back_in_the_list_endpoints_shape() {
    let user = test_user();
    let relationship_id = Id::new_v4();
    let earlier = session(relationship_id);
    let db = logged_in(&user)
        .append_query_results([vec![relationship(relationship_id, user.id)]])
        .append_query_results(vec![vec![earlier.clone()], vec![]])
        // View markers, first topic bodies, first goal titles.
        .append_query_results::<coaching_sessions::Model, Vec<_>, _>(vec![vec![], vec![], vec![]])
        .into_connection();
    let app = build_app(Arc::new(db));

    let response = get_nearest(&app, user.id, &window_query(relationship_id)).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let [only] = body["data"]
        .as_array()
        .expect("data is an array")
        .as_slice()
    else {
        panic!("expected one session, got {body}");
    };
    assert_eq!(only["id"], json!(earlier.id));
    assert_eq!(only["date"], json!("2026-10-29T14:00:00"));
    assert_eq!(only["display_title"], json!("Quarterly review"));
    assert!(only
        .get("viewer_last_viewed_at")
        .is_some_and(Value::is_null));
}

/// `include` reaches the query: `organization` without `relationship` is refused
/// there, before any session is read.
#[tokio::test]
async fn include_is_passed_through_to_the_query() {
    let user = test_user();
    let app = build_app(Arc::new(logged_in(&user).into_connection()));

    let query = format!("{}&include=organization", window_query(Id::new_v4()));
    let response = get_nearest(&app, user.id, &query).await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// An unknown `include` value is rejected the way the list endpoint rejects it.
#[tokio::test]
async fn an_unknown_include_is_a_plain_400_like_the_list_endpoint() {
    let user = test_user();
    let app = build_app(Arc::new(logged_in(&user).into_connection()));

    let query = format!("{}&include=bogus", window_query(Id::new_v4()));
    let response = get_nearest(&app, user.id, &query).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let content_type = response.headers().get("content-type").cloned();
    assert!(content_type.is_some_and(|c| c.to_str().unwrap_or("").starts_with("text/plain")));
}
