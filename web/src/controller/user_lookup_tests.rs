use crate::controller::user_controller::index;
use crate::middleware::auth::require_auth;
use crate::AppState;
use axum::http::StatusCode;
use axum::{body::Body, extract::Request, middleware::from_fn, routing::get, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::Utc;
use domain::user::Backend;
use domain::{user_roles, users, Id};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult, Value};
use service::config::Config;
use std::collections::BTreeMap;
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

fn target(id: Id) -> users::Model {
    users::Model {
        id,
        email: "found@example.com".to_string(),
        first_name: "Found".to_string(),
        last_name: "Person".to_string(),
        ..requester()
    }
}

fn test_role(user_id: Id, organization_id: Option<Id>, role: users::Role) -> user_roles::Model {
    let now = Utc::now();
    user_roles::Model {
        id: Id::new_v4(),
        role,
        organization_id,
        user_id,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn build_app(db: Arc<sea_orm::DatabaseConnection>) -> Router {
    let app_state = AppState::new(
        service::AppState::new(Config::default(), &db),
        Arc::new(sse::Manager::default()),
        domain::events::EventPublisher::default(),
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
                .route("/users", get(index))
                .route_layer(from_fn(require_auth)),
        )
        .layer(auth_layer)
        .with_state(app_state)
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

async fn lookup(app: &Router, cookie: &str, email: &str) -> axum::response::Response {
    let request = Request::builder()
        .uri(format!("/users?email={email}"))
        .header("cookie", cookie)
        .header("x-version", "1.0.0-beta1")
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

/// The `data` array of an `ApiResponse` body.
async fn data_array(response: axum::response::Response) -> Vec<serde_json::Value> {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    body["data"]
        .as_array()
        .expect("data must be a JSON array")
        .clone()
}

/// Empty rows for both visibility probes: `shares_administered_organization` and
/// `was_member_of_administered_organization`, two queries each.
///
/// Both run on every path, including unknown emails, so response timing cannot
/// separate "no such email" from "not yours to see".
fn empty_scope_probe(mock: MockDatabase) -> MockDatabase {
    (0..4).fold(mock, |mock, _| {
        mock.append_query_results([Vec::<BTreeMap<String, Value>>::new()])
    })
}

/// The rate-limit gate ahead of the lookup: an advisory lock, a count, then the
/// recorded attempt. The lock is an exec, which `MockDatabase` queues separately
/// from query results.
fn under_the_rate_limit(mock: MockDatabase) -> MockDatabase {
    mock.append_exec_results([MockExecResult {
        last_insert_id: 0,
        rows_affected: 0,
    }])
    .append_query_results([Vec::<domain::user_lookup_attempts::Model>::new()])
    .append_query_results([[domain::user_lookup_attempts::Model {
        id: Id::new_v4(),
        requester_user_id: Id::new_v4(),
        attempted_at: chrono::Utc::now().into(),
    }]])
}

#[tokio::test]
async fn lookup_returns_403_for_a_requester_who_administers_nothing() {
    let user = requester();
    let role = test_role(user.id, Some(Id::new_v4()), users::Role::User);

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = lookup(&app, &cookie, "found@example.com").await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn lookup_returns_200_with_one_element_for_a_super_admin() {
    let user = requester();
    let role = test_role(user.id, None, users::Role::SuperAdmin);
    let target_id = Id::new_v4();

    let mock = under_the_rate_limit(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]]),
    )
    .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
        target(target_id),
        None,
    )]]);

    let app = build_app(Arc::new(empty_scope_probe(mock).into_connection()));
    let cookie = login_cookie(&app).await;

    let response = lookup(&app, &cookie, "found@example.com").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(data_array(response).await.len(), 1);
}

#[tokio::test]
async fn lookup_returns_200_and_an_empty_array_when_nothing_matches() {
    let user = requester();
    let role = test_role(user.id, None, users::Role::SuperAdmin);

    let mock = under_the_rate_limit(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]]),
    )
    .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![]]);

    let app = build_app(Arc::new(empty_scope_probe(mock).into_connection()));
    let cookie = login_cookie(&app).await;

    let response = lookup(&app, &cookie, "nobody@example.com").await;
    // An empty array is the only not-found signal; a 404 here would be an
    // email-enumeration oracle.
    assert_eq!(response.status(), StatusCode::OK);
    assert!(data_array(response).await.is_empty());
}

#[tokio::test]
async fn lookup_result_carries_only_the_narrow_dto_fields() {
    let user = requester();
    let role = test_role(user.id, None, users::Role::SuperAdmin);
    let target_id = Id::new_v4();

    let mock = under_the_rate_limit(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]]),
    )
    .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
        target(target_id),
        None,
    )]]);

    let app = build_app(Arc::new(empty_scope_probe(mock).into_connection()));
    let cookie = login_cookie(&app).await;

    let response = lookup(&app, &cookie, "found@example.com").await;
    assert_eq!(response.status(), StatusCode::OK);

    let results = data_array(response).await;
    let found = results[0].as_object().expect("element must be an object");

    let mut keys: Vec<&str> = found.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["email", "first_name", "id", "last_name"]);
    assert!(
        !found.contains_key("roles"),
        "the lookup DTO must never leak the target's roles"
    );
}

/// I-C4. Over the allowance the endpoint must refuse before it looks anything up,
/// or the throttle is decorative: the work it exists to prevent has already happened
/// by the time the 429 is written.
///
/// The mock is primed with the count and nothing else, so a handler that proceeded
/// would run past the end of the queue and surface as a 500. A 429 here can only
/// mean the lookup never ran.
#[tokio::test]
async fn lookup_returns_429_over_the_rate_limit_without_looking_anything_up() {
    let user = requester();
    let role = test_role(user.id, Some(Id::new_v4()), users::Role::Admin);

    let over_the_limit = (0..domain::user_lookup::MAX_ATTEMPTS_PER_WINDOW)
        .map(|_| domain::user_lookup_attempts::Model {
            id: Id::new_v4(),
            requester_user_id: user.id,
            attempted_at: chrono::Utc::now().into(),
        })
        .collect::<Vec<_>>();

    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .append_query_results([over_the_limit])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = lookup(&app, &cookie, "found@example.com").await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
}

/// The complement: at one below the allowance the lookup proceeds normally. Without
/// this, a gate that refused everyone would satisfy the test above.
#[tokio::test]
async fn lookup_proceeds_under_the_rate_limit() {
    let user = requester();
    let role = test_role(user.id, None, users::Role::SuperAdmin);
    let target_id = Id::new_v4();

    let mock = under_the_rate_limit(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user.clone(), role.clone())]]),
    )
    .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
        target(target_id),
        None,
    )]]);

    let app = build_app(Arc::new(empty_scope_probe(mock).into_connection()));
    let cookie = login_cookie(&app).await;

    let response = lookup(&app, &cookie, "found@example.com").await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(data_array(response).await.len(), 1);
}
