use crate::controller::organization::user_controller::{attach_role, remove_role};
use crate::middleware::auth::require_auth;
use crate::AppState;
use axum::http::StatusCode;
use axum::{body::Body, extract::Request, middleware::from_fn, routing::post, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::Utc;
use domain::user::Backend;
use domain::{organizations, user_roles, users, Id};
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
        role: users::Role::User,
        roles: vec![],
        invite_status: None,
        created_at: now.into(),
        updated_at: now.into(),
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

fn test_organization(organization_id: Id) -> organizations::Model {
    let now = Utc::now();
    organizations::Model {
        id: organization_id,
        name: "Refactor Group".to_owned(),
        slug: "refactor-group".to_owned(),
        logo: None,
        created_at: now.into(),
        updated_at: now.into(),
        archived_at: None,
        archived_by: None,
    }
}

/// A single-column mock row, for the id-only selects the visibility probe runs.
fn count_row(count: i64) -> BTreeMap<String, Value> {
    BTreeMap::from([("num_items".to_string(), count.into())])
}

fn id_row(column: &str, value: Id) -> BTreeMap<String, Value> {
    BTreeMap::from([(column.to_string(), value.into())])
}

fn exec_result() -> MockExecResult {
    MockExecResult {
        last_insert_id: 0,
        rows_affected: 1,
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
                .route(
                    "/organizations/:organization_id/users/:user_id/role",
                    post(attach_role).delete(remove_role),
                )
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

async fn role_request(
    app: &Router,
    cookie: &str,
    method: &str,
    organization_id: Id,
    user_id: Id,
    body: Body,
) -> axum::response::Response {
    let request = Request::builder()
        .uri(format!(
            "/organizations/{organization_id}/users/{user_id}/role"
        ))
        .method(method)
        .header("cookie", cookie)
        .header("x-version", "1.0.0-beta1")
        .header("content-type", "application/json")
        .body(body)
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

/// The `status_code` the `ApiResponse` envelope reports. Success responses in this
/// codebase are HTTP 200 and carry their real code in the body.
async fn api_status_code(response: axum::response::Response) -> u64 {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["status_code"]
        .as_u64()
        .expect("envelope must carry a status_code")
}

/// Login, session and the extractor's organization lookup: the three queries every
/// one of these routes runs before its own logic starts.
fn mock_through_extractor(
    user: &users::Model,
    role: &user_roles::Model,
    organization_id: Id,
) -> MockDatabase {
    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![(user.clone(), role.clone())]])
        .append_query_results([vec![(user.clone(), role.clone())]])
        .append_query_results([vec![test_organization(organization_id)]])
}

#[tokio::test]
async fn attach_returns_404_for_a_user_the_admin_cannot_see() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    // The visibility probe finds an administered org but no overlap, so it answers
    // false. Its rows are id-only, unreadable as an organization: skipping the probe
    // feeds them to attach_to_organization's first query and surfaces as a 500, so a
    // 404 here can only have come from the visibility check.
    let db = Arc::new(
        mock_through_extractor(&user, &role, organization_id)
            .append_query_results([vec![id_row("organization_id", organization_id)]])
            .append_query_results([Vec::<BTreeMap<String, Value>>::new()])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "POST",
        organization_id,
        Id::new_v4(),
        Body::from(r#"{"role":"User"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn attach_returns_201_for_an_org_admin_who_can_see_the_target() {
    // The driving use case: an admin of two organizations moving a member of one
    // into the other. Distinct from the super admin path, which short-circuits the
    // visibility probe entirely, so that test cannot catch org admins losing this.
    let organization_id = Id::new_v4();
    let target_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);
    let granted = test_role(target_id, Some(organization_id), users::Role::User);

    let db = Arc::new(
        mock_through_extractor(&user, &role, organization_id)
            // Visibility probe: an administered org, and the target is in it.
            .append_query_results([vec![id_row("organization_id", organization_id)]])
            .append_query_results([vec![id_row("id", Id::new_v4())]])
            .append_query_results([vec![test_organization(organization_id)]])
            .append_query_results([vec![requester()]])
            .append_query_results([Vec::<user_roles::Model>::new()])
            .append_query_results([vec![granted.clone()]])
            .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
                requester(),
                Some(granted),
            )]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "POST",
        organization_id,
        target_id,
        Body::from(r#"{"role":"User"}"#),
    )
    .await;

    assert_eq!(api_status_code(response).await, 201);
}

#[tokio::test]
async fn attach_returns_409_for_an_org_admin_whose_only_visible_users_are_members() {
    // The other half of the visibility rule. An org admin can only see users in
    // organizations they administer, so for an admin of a single organization every
    // visible user is already a member and attach can only ever conflict. Pins the
    // mechanism behind that; the graph-level claim is exercised live, not here.
    let organization_id = Id::new_v4();
    let target_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(
        mock_through_extractor(&user, &role, organization_id)
            .append_query_results([vec![id_row("organization_id", organization_id)]])
            .append_query_results([vec![id_row("id", Id::new_v4())]])
            .append_query_results([vec![test_organization(organization_id)]])
            .append_query_results([vec![requester()]])
            // Already holds a role here, which is the only state a single-org
            // admin's visible users can be in.
            .append_query_results([vec![test_role(
                target_id,
                Some(organization_id),
                users::Role::User,
            )]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "POST",
        organization_id,
        target_id,
        Body::from(r#"{"role":"User"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn attach_returns_201_for_a_super_admin() {
    let organization_id = Id::new_v4();
    let target_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, None, users::Role::SuperAdmin);
    let granted = test_role(target_id, Some(organization_id), users::Role::User);

    // A super admin short-circuits the visibility probe, so the next queries are
    // attach_to_organization's own.
    let db = Arc::new(
        mock_through_extractor(&user, &role, organization_id)
            .append_query_results([vec![test_organization(organization_id)]])
            .append_query_results([vec![requester()]])
            .append_query_results([Vec::<user_roles::Model>::new()])
            .append_query_results([vec![granted.clone()]])
            .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
                requester(),
                Some(granted),
            )]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "POST",
        organization_id,
        target_id,
        Body::from(r#"{"role":"User"}"#),
    )
    .await;

    assert_eq!(api_status_code(response).await, 201);
}

#[tokio::test]
async fn attach_rejects_a_super_admin_role_in_the_body() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, None, users::Role::SuperAdmin);

    let db = Arc::new(mock_through_extractor(&user, &role, organization_id).into_connection());

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "POST",
        organization_id,
        Id::new_v4(),
        Body::from(r#"{"role":"SuperAdmin"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn attach_rejects_an_unknown_field_in_the_body() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(mock_through_extractor(&user, &role, organization_id).into_connection());

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "POST",
        organization_id,
        Id::new_v4(),
        Body::from(r#"{"role":"User","organization_id":"whatever"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn remove_returns_403_when_the_requester_targets_themselves() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(mock_through_extractor(&user, &role, organization_id).into_connection());

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "DELETE",
        organization_id,
        user.id,
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn remove_returns_409_when_the_member_still_has_sessions() {
    let organization_id = Id::new_v4();
    let target_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(
        mock_through_extractor(&user, &role, organization_id)
            .append_query_results([vec![test_role(
                target_id,
                Some(organization_id),
                users::Role::User,
            )]])
            .append_query_results([vec![id_row("id", relationship_id)]])
            .append_query_results([vec![count_row(2)]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "DELETE",
        organization_id,
        target_id,
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn remove_returns_204_for_another_member() {
    let organization_id = Id::new_v4();
    let target_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(
        mock_through_extractor(&user, &role, organization_id)
            .append_query_results([vec![test_role(
                target_id,
                Some(organization_id),
                users::Role::User,
            )]])
            // No coaching relationships, so no session count and nothing blocking.
            .append_query_results([Vec::<BTreeMap<String, sea_orm::Value>>::new()])
            .append_exec_results([exec_result(), exec_result()])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "DELETE",
        organization_id,
        target_id,
        Body::empty(),
    )
    .await;

    assert_eq!(api_status_code(response).await, 204);
}

/// Nothing else pins that the handler forwards the body's coach into the domain
/// call: hardcoding `None` at the call site passes every other test in the suite.
/// The mock is primed only for the coachless path, so a forwarded coach must run
/// past the end of the queue and fail the request.
#[tokio::test]
async fn attach_forwards_the_requested_coach_to_the_domain_call() {
    let organization_id = Id::new_v4();
    let target_id = Id::new_v4();
    let coach_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, None, users::Role::SuperAdmin);
    let granted = test_role(target_id, Some(organization_id), users::Role::User);

    let db = Arc::new(
        mock_through_extractor(&user, &role, organization_id)
            .append_query_results([vec![test_organization(organization_id)]])
            .append_query_results([vec![requester()]])
            .append_query_results([Vec::<user_roles::Model>::new()])
            .append_query_results([vec![granted.clone()]])
            .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
                requester(),
                Some(granted),
            )]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "POST",
        organization_id,
        target_id,
        Body::from(format!(r#"{{"role":"User","coach_id":"{coach_id}"}}"#)),
    )
    .await;

    assert_ne!(
        response.status(),
        StatusCode::OK,
        "a coach in the body must reach the domain call, not be dropped"
    );
}
