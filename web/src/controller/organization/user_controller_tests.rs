//! Authorization regression tests for the organization user routes.
//!
//! The routes are exercised through the real router so the fixtures stay valid
//! whichever mechanism gates them.

use crate::AppState;
use axum::http::StatusCode;
use axum::{body::Body, extract::Request, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::Utc;
use domain::token_purpose::TokenPurpose;
use domain::user::Backend;
use domain::{magic_link_tokens, organizations, user_role_changes, user_roles, users, Id};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, IntoMockRow, MockDatabase, MockExecResult, MockRow, Value};
use service::config::Config;
use std::collections::BTreeMap;
use std::sync::Arc;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

const API_VERSION: &str = "1.0.0-beta1";

const NEW_USER_BODY: &str = r#"{
    "email": "new@example.com",
    "first_name": "New",
    "last_name": "Member",
    "display_name": null,
    "password": "secret123",
    "github_username": null,
    "github_profile_url": null
}"#;

fn user_with(email: &str, password: Option<String>) -> users::Model {
    let now = Utc::now();
    users::Model {
        id: Id::new_v4(),
        email: email.to_string(),
        first_name: "Test".to_string(),
        last_name: "User".to_string(),
        display_name: Some("Test User".to_string()),
        password,
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

/// The logged-in caller. The password matches the credentials `login_cookie` posts.
fn requester() -> users::Model {
    user_with("test@example.com", Some(generate_hash("password123")))
}

/// A second member of the organization, still awaiting setup so `resend_invite`
/// does not reject it.
fn target_user() -> users::Model {
    user_with("target@example.com", None)
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

fn test_magic_link_token(user_id: Id) -> magic_link_tokens::Model {
    let now = Utc::now();
    magic_link_tokens::Model {
        id: Id::new_v4(),
        user_id,
        token_hash: "hash".to_string(),
        expires_at: now.into(),
        created_at: now.into(),
        purpose: TokenPurpose::Setup,
    }
}

fn exec_result() -> MockExecResult {
    MockExecResult {
        last_insert_id: 0,
        rows_affected: 1,
    }
}

fn row(item: impl IntoMockRow) -> MockRow {
    item.into_mock_row()
}

/// The `COUNT(*)` shape SeaORM reads for a Postgres count query.
fn count_row(count: i64) -> MockRow {
    BTreeMap::from([("num_items".to_string(), Value::BigInt(Some(count)))]).into_mock_row()
}

/// Folds rows into one; later rows win on shared column names.
///
/// The middleware and the extractor issue the same queries in slightly different
/// orders, so a row that only fits one order would pin the wiring rather than the
/// behavior.
fn merge(rows: impl IntoIterator<Item = MockRow>) -> BTreeMap<String, Value> {
    rows.into_iter()
        .flat_map(MockRow::into_column_value_tuples)
        .collect()
}

/// The audit row `create_by_organization` appends alongside the default role.
fn role_change_row(
    actor_user_id: Id,
    target_user_id: Id,
    organization_id: Id,
) -> user_role_changes::Model {
    user_role_changes::Model {
        id: Id::new_v4(),
        actor_user_id: Some(actor_user_id),
        target_user_id,
        organization_id: Some(organization_id),
        previous_role: None,
        new_role: Some(users::Role::User),
        changed_at: chrono::Utc::now().into(),
    }
}

/// The two user lookups every request makes: login, then the session load.
fn authenticated_as(user: &users::Model, role: &user_roles::Model) -> MockDatabase {
    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![merge([row((user.clone(), role.clone()))])]])
        .append_query_results([vec![merge([row((user.clone(), role.clone()))])]])
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

    crate::router::define_routes(app_state).layer(auth_layer)
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

async fn send(
    app: &Router,
    cookie: &str,
    method: &str,
    uri: String,
    body: Body,
) -> axum::response::Response {
    let request = Request::builder()
        .uri(uri)
        .method(method)
        .header("cookie", cookie)
        .header("x-version", API_VERSION)
        .header("content-type", "application/json")
        .body(body)
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

/// The `status_code` the `ApiResponse` envelope reports. Success responses in this
/// codebase are HTTP 200 and carry their real code in the body.
async fn api_status_code(response: axum::response::Response) -> u64 {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["status_code"]
        .as_u64()
        .expect("envelope must carry a status_code")
}

fn create_uri(organization_id: Id) -> String {
    format!("/organizations/{organization_id}/users")
}

fn resend_invite_uri(organization_id: Id, user_id: Id) -> String {
    format!("/organizations/{organization_id}/users/{user_id}/resend-invite")
}

fn delete_uri(organization_id: Id, user_id: Id) -> String {
    format!("/organizations/{organization_id}/users/{user_id}")
}

#[tokio::test]
async fn create_is_forbidden_for_a_plain_member() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::User);

    let db = Arc::new(
        authenticated_as(&user, &role)
            .append_query_results([vec![merge([row(test_organization(organization_id))])]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = send(
        &app,
        &cookie,
        "POST",
        create_uri(organization_id),
        Body::from(NEW_USER_BODY),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn create_succeeds_for_an_organization_admin() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);
    let organization = test_organization(organization_id);
    let new_user = target_user();
    let new_role = test_role(new_user.id, Some(organization_id), users::Role::User);

    let db = Arc::new(
        authenticated_as(&user, &role)
            .append_query_results([vec![merge([row(organization.clone())])]])
            .append_query_results([vec![merge([
                row((user.clone(), role.clone())),
                row(organization.clone()),
            ])]])
            .append_query_results([vec![merge([
                row(organization.clone()),
                row(new_user.clone()),
            ])]])
            .append_query_results([vec![merge([row(new_user.clone()), row(new_role.clone())])]])
            // The org admin path consumes one more result than the super admin one,
            // so the audit row is both merged here and appended below.
            .append_query_results([vec![merge([
                row(new_role.clone()),
                row(role_change_row(user.id, new_user.id, organization_id)),
            ])]])
            .append_query_results([vec![merge([row(role_change_row(
                user.id,
                new_user.id,
                organization_id,
            ))])]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = send(
        &app,
        &cookie,
        "POST",
        create_uri(organization_id),
        Body::from(NEW_USER_BODY),
    )
    .await;

    assert_eq!(api_status_code(response).await, 201);
}

#[tokio::test]
async fn create_succeeds_for_a_super_admin() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, None, users::Role::SuperAdmin);
    let organization = test_organization(organization_id);
    let new_user = target_user();
    let new_role = test_role(new_user.id, Some(organization_id), users::Role::User);

    let db = Arc::new(
        authenticated_as(&user, &role)
            .append_query_results([vec![merge([row(organization.clone())])]])
            .append_query_results([vec![merge([row(organization.clone())])]])
            .append_query_results([vec![merge([row(new_user.clone())])]])
            .append_query_results([vec![merge([row(new_role.clone())])]])
            .append_query_results([vec![merge([row(role_change_row(
                user.id,
                new_user.id,
                organization_id,
            ))])]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = send(
        &app,
        &cookie,
        "POST",
        create_uri(organization_id),
        Body::from(NEW_USER_BODY),
    )
    .await;

    assert_eq!(api_status_code(response).await, 201);
}

#[tokio::test]
async fn resend_invite_is_forbidden_for_a_plain_member() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::User);
    let target = target_user();

    let db = Arc::new(
        authenticated_as(&user, &role)
            .append_query_results([vec![merge([row(test_organization(organization_id))])]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = send(
        &app,
        &cookie,
        "POST",
        resend_invite_uri(organization_id, target.id),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn resend_invite_reaches_the_handler_for_an_organization_admin() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);
    let target = target_user();
    let target_role = test_role(target.id, Some(organization_id), users::Role::User);

    let token = test_magic_link_token(target.id);
    let members = vec![
        merge([row((user.clone(), role.clone()))]),
        merge([row((target.clone(), target_role.clone()))]),
    ];
    let members_with_token = vec![
        merge([row((user.clone(), role.clone())), row(token.clone())]),
        merge([row((target.clone(), target_role.clone()))]),
    ];

    let db = Arc::new(
        authenticated_as(&user, &role)
            .append_query_results([vec![merge([row(test_organization(organization_id))])]])
            .append_query_results([members])
            .append_query_results([members_with_token])
            .append_query_results([vec![merge([row(token)])]])
            .append_exec_results([exec_result()])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = send(
        &app,
        &cookie,
        "POST",
        resend_invite_uri(organization_id, target.id),
        Body::empty(),
    )
    .await;

    // The invite email cannot be delivered under a default test config, so the
    // handler's own email failure is the proof authorization let the request through.
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn delete_is_forbidden_for_a_plain_member() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::User);
    let target = target_user();

    let db = Arc::new(
        authenticated_as(&user, &role)
            .append_query_results([vec![merge([row(test_organization(organization_id))])]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = send(
        &app,
        &cookie,
        "DELETE",
        delete_uri(organization_id, target.id),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_succeeds_for_an_organization_admin_targeting_another_member() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);
    let target = target_user();
    let target_role = test_role(target.id, Some(organization_id), users::Role::User);

    let requester_row = merge([row((user.clone(), role.clone()))]);
    let target_row = merge([row((target.clone(), target_role.clone()))]);
    let requester_row_with_count = merge([row((user.clone(), role.clone())), count_row(1)]);

    let db = Arc::new(
        authenticated_as(&user, &role)
            .append_query_results([vec![merge([row(test_organization(organization_id))])]])
            .append_query_results([vec![requester_row, target_row.clone()]])
            // The lock the deletion takes on the target before counting its orgs.
            .append_query_results([vec![merge([row(target.clone())])]])
            .append_query_results([vec![requester_row_with_count, target_row]])
            .append_query_results([vec![merge([count_row(1)])]])
            // The roles the deletion is about to destroy, read before they are gone,
            // then the audit row each one owes.
            .append_query_results([vec![merge([row(target_role.clone())])]])
            .append_query_results([vec![merge([row(role_change_row(
                user.id,
                target.id,
                organization_id,
            ))])]])
            .append_exec_results([exec_result(), exec_result(), exec_result()])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = send(
        &app,
        &cookie,
        "DELETE",
        delete_uri(organization_id, target.id),
        Body::empty(),
    )
    .await;

    assert_eq!(api_status_code(response).await, 204);
}

#[tokio::test]
async fn delete_is_forbidden_for_an_organization_admin_targeting_themselves() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(
        authenticated_as(&user, &role)
            .append_query_results([vec![merge([row(test_organization(organization_id))])]])
            .append_query_results([vec![merge([row((user.clone(), role.clone()))])]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = send(
        &app,
        &cookie,
        "DELETE",
        delete_uri(organization_id, user.id),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn every_route_is_forbidden_for_an_admin_of_another_organization() {
    let organization_id = Id::new_v4();
    let target = target_user();

    let requests = [
        ("POST", create_uri(organization_id), NEW_USER_BODY),
        ("POST", resend_invite_uri(organization_id, target.id), "{}"),
        ("DELETE", delete_uri(organization_id, target.id), "{}"),
    ];

    for (method, uri, body) in requests {
        let user = requester();
        let role = test_role(user.id, Some(Id::new_v4()), users::Role::Admin);

        let db = Arc::new(
            authenticated_as(&user, &role)
                .append_query_results([vec![merge([row(test_organization(organization_id))])]])
                .into_connection(),
        );

        let app = build_app(db);
        let cookie = login_cookie(&app).await;

        let response = send(&app, &cookie, method, uri, Body::from(body)).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} route");
    }
}
