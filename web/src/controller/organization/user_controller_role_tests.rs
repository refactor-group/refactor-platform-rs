use crate::controller::organization::user_controller::{
    attach_role, read_role, remove_role, update_role,
};
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
use domain::{organizations, user_role_changes, user_roles, users, Id};
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
fn id_row(column: &str, value: Id) -> BTreeMap<String, Value> {
    BTreeMap::from([(column.to_string(), value.into())])
}

/// The audit row a grant or removal appends inside the domain transaction.
fn role_change_row(
    actor_user_id: Id,
    target_user_id: Id,
    organization_id: Id,
    previous_role: Option<users::Role>,
    new_role: Option<users::Role>,
) -> user_role_changes::Model {
    user_role_changes::Model {
        id: Id::new_v4(),
        actor_user_id: Some(actor_user_id),
        target_user_id,
        organization_id: Some(organization_id),
        previous_role,
        new_role,
        changed_at: chrono::Utc::now().into(),
    }
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
                    get(read_role)
                        .post(attach_role)
                        .put(update_role)
                        .delete(remove_role),
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

/// The envelope's `status_code` and its `data`, so a test can assert what came back
/// rather than only that something did. [`api_status_code`] discards the payload,
/// which makes a status assertion easy to mistake for a content one.
async fn api_response(response: axum::response::Response) -> (u64, serde_json::Value) {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let status = body["status_code"]
        .as_u64()
        .expect("envelope must carry a status_code");
    (status, body["data"].clone())
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
            .append_query_results([vec![role_change_row(
                user.id,
                target_id,
                organization_id,
                None,
                Some(users::Role::User),
            )]])
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
            .append_query_results([vec![role_change_row(
                user.id,
                target_id,
                organization_id,
                None,
                Some(users::Role::User),
            )]])
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
async fn remove_returns_204_when_the_member_still_has_sessions() {
    let organization_id = Id::new_v4();
    let target_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    // The member's coaching history no longer factors into removal, so the
    // handler runs the membership lookup and the one delete regardless.
    let db = Arc::new(
        mock_through_extractor(&user, &role, organization_id)
            .append_query_results([vec![test_role(
                target_id,
                Some(organization_id),
                users::Role::User,
            )]])
            .append_exec_results([exec_result()])
            .append_query_results([vec![role_change_row(
                user.id,
                target_id,
                organization_id,
                Some(users::Role::User),
                None,
            )]])
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
            .append_exec_results([exec_result()])
            .append_query_results([vec![role_change_row(
                user.id,
                target_id,
                organization_id,
                Some(users::Role::User),
                None,
            )]])
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

/// The queries the PUT runs after the extractor, for a change allowed to proceed.
/// The domain layer re-reads the organization; the extractor's own lookup is a
/// separate statement and does not stand in for it.
fn mock_update_through_domain(
    mock: MockDatabase,
    actor_id: Id,
    organization_id: Id,
    target_id: Id,
    current: users::Role,
    requested: users::Role,
) -> MockDatabase {
    let updated = test_role(target_id, Some(organization_id), requested.clone());

    mock.append_query_results([vec![test_organization(organization_id)]])
        .append_query_results([vec![requester()]])
        .append_query_results([vec![test_role(
            target_id,
            Some(organization_id),
            current.clone(),
        )]])
        .append_query_results([vec![updated.clone()]])
        .append_query_results([vec![role_change_row(
            actor_id,
            target_id,
            organization_id,
            Some(current),
            Some(requested),
        )]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([vec![(
            requester(),
            Some(updated),
        )]])
}

/// I-1. The refusal must happen in the extractor, before the handler body runs at
/// all. The mock is primed only for the extractor, so a handler that reached its own
/// logic would run past the end of the queue and surface as a 500: a 403 here can
/// only mean the target user was never looked up.
#[tokio::test]
async fn update_refuses_a_plain_member_before_reaching_the_handler_body() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::User);

    let db = Arc::new(mock_through_extractor(&user, &role, organization_id).into_connection());

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "PUT",
        organization_id,
        Id::new_v4(),
        Body::from(r#"{"role":"Admin"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// The read is gated the same way as the write. A plain member of the organization
/// can already see who its admins are through the member list, so this endpoint is
/// stricter than that one, deliberately.
#[tokio::test]
async fn read_refuses_a_plain_member_before_reaching_the_handler_body() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::User);

    let db = Arc::new(mock_through_extractor(&user, &role, organization_id).into_connection());

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "GET",
        organization_id,
        Id::new_v4(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// I-2. An unauthorized caller must learn nothing about the target, so the four
/// combinations of caller and target are compared against each other rather than
/// asserted one by one. A future change that makes any single branch answer 404
/// fails on the comparison, which is the property worth defending: were a real user
/// id to answer differently from a fabricated one, any org admin could enumerate
/// platform user ids.
#[tokio::test]
async fn unauthorized_callers_cannot_tell_whether_the_target_exists() {
    let organization_id = Id::new_v4();
    let real_target = Id::new_v4();
    let fabricated_target = Id::nil();

    let mut statuses = Vec::new();
    for caller_role in [
        users::Role::User,
        users::Role::Admin, // of a different organization
    ] {
        let caller_organization = match caller_role {
            users::Role::Admin => Id::new_v4(),
            _ => organization_id,
        };

        for target in [real_target, fabricated_target] {
            let user = requester();
            let role = test_role(user.id, Some(caller_organization), caller_role.clone());
            let db =
                Arc::new(mock_through_extractor(&user, &role, organization_id).into_connection());

            let app = build_app(db);
            let cookie = login_cookie(&app).await;

            let response = role_request(
                &app,
                &cookie,
                "PUT",
                organization_id,
                target,
                Body::from(r#"{"role":"Admin"}"#),
            )
            .await;

            statuses.push(response.status());
        }
    }

    assert_eq!(
        statuses,
        vec![StatusCode::FORBIDDEN; 4],
        "every unauthorized caller must get the same refusal regardless of target"
    );
}

/// I-4. An authorized admin still must not learn whether a user exists outside their
/// organization. Both cases resolve to one org-scoped lookup that finds nothing, so
/// the test's real job is to fail if a second check is ever added that answers them
/// differently.
#[tokio::test]
async fn an_admin_cannot_tell_a_missing_user_from_one_in_another_organization() {
    let organization_id = Id::new_v4();

    async fn status_and_body(organization_id: Id, target: Id) -> (StatusCode, Vec<u8>) {
        let user = requester();
        let role = test_role(user.id, Some(organization_id), users::Role::Admin);
        let db = Arc::new(
            mock_through_extractor(&user, &role, organization_id)
                .append_query_results([Vec::<user_roles::Model>::new()])
                .into_connection(),
        );

        let app = build_app(db);
        let cookie = login_cookie(&app).await;

        let response =
            role_request(&app, &cookie, "GET", organization_id, target, Body::empty()).await;
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        (status, body)
    }

    let missing = status_and_body(organization_id, Id::nil()).await;
    let elsewhere = status_and_body(organization_id, Id::new_v4()).await;

    assert_eq!(
        missing, elsewhere,
        "a missing user and an invisible one must be byte-identical"
    );
    assert_eq!(
        missing.0,
        StatusCode::NOT_FOUND,
        "and the shared answer must be the uninformative 404"
    );
}

/// I-5. Self-targeting is refused in both directions. Blocking only demotion would
/// leave an admin able to re-grant themselves a role they had lost, and the
/// direction of a change is not what makes self-targeting wrong.
#[tokio::test]
async fn update_refuses_a_requester_targeting_themselves_in_either_direction() {
    for requested in ["Admin", "User"] {
        let organization_id = Id::new_v4();
        let user = requester();
        let role = test_role(user.id, Some(organization_id), users::Role::Admin);

        let db = Arc::new(mock_through_extractor(&user, &role, organization_id).into_connection());

        let app = build_app(db);
        let cookie = login_cookie(&app).await;

        let response = role_request(
            &app,
            &cookie,
            "PUT",
            organization_id,
            user.id,
            Body::from(format!(r#"{{"role":"{requested}"}}"#)),
        )
        .await;

        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "self-targeting must be refused when requesting {requested}"
        );
    }
}

#[tokio::test]
async fn read_returns_the_members_role() {
    let organization_id = Id::new_v4();
    let target_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(
        mock_through_extractor(&user, &role, organization_id)
            .append_query_results([vec![test_role(
                target_id,
                Some(organization_id),
                users::Role::Admin,
            )]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "GET",
        organization_id,
        target_id,
        Body::empty(),
    )
    .await;

    let (status, membership) = api_response(response).await;
    assert_eq!(status, 200);
    // The status alone would pass for any membership the handler happened to return.
    assert_eq!(membership["role"], "Admin");
    assert_eq!(membership["user_id"], target_id.to_string());
    assert_eq!(membership["organization_id"], organization_id.to_string());
}

#[tokio::test]
async fn update_promotes_a_member_to_admin() {
    let organization_id = Id::new_v4();
    let target_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(
        mock_update_through_domain(
            mock_through_extractor(&user, &role, organization_id),
            user.id,
            organization_id,
            target_id,
            users::Role::User,
            users::Role::Admin,
        )
        .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "PUT",
        organization_id,
        target_id,
        Body::from(r#"{"role":"Admin"}"#),
    )
    .await;

    let (status, user) = api_response(response).await;
    assert_eq!(status, 200);
    // Emptying the roles array passes a status-only assertion, so the payload has to
    // be read: the granted role is what the caller renders the row from.
    assert_eq!(user["roles"][0]["role"], "Admin");
    assert_eq!(
        user["roles"][0]["organization_id"],
        organization_id.to_string()
    );
}

/// A super admin holds no membership in the organization, so this path proves the
/// endpoint does not require the caller to be a member of the organization they are
/// administering.
#[tokio::test]
async fn update_promotes_a_member_for_a_super_admin() {
    let organization_id = Id::new_v4();
    let target_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, None, users::Role::SuperAdmin);

    let db = Arc::new(
        mock_update_through_domain(
            mock_through_extractor(&user, &role, organization_id),
            user.id,
            organization_id,
            target_id,
            users::Role::User,
            users::Role::Admin,
        )
        .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "PUT",
        organization_id,
        target_id,
        Body::from(r#"{"role":"Admin"}"#),
    )
    .await;

    let (status, user) = api_response(response).await;
    assert_eq!(status, 200);
    assert_eq!(user["roles"][0]["role"], "Admin");
}

/// The 422 must come from the handler, before the domain call. The mock is primed
/// only for the extractor, so a request that reached the domain layer would surface
/// as a 500 instead.
#[tokio::test]
async fn update_rejects_a_super_admin_role_in_the_body() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(mock_through_extractor(&user, &role, organization_id).into_connection());

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "PUT",
        organization_id,
        Id::new_v4(),
        Body::from(r#"{"role":"SuperAdmin"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn update_rejects_an_unknown_field_in_the_body() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(mock_through_extractor(&user, &role, organization_id).into_connection());

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "PUT",
        organization_id,
        Id::new_v4(),
        Body::from(format!(
            r#"{{"role":"Admin","coach_id":"{}"}}"#,
            Id::new_v4()
        )),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// Reaching the last-admin guard proves the whole stack is wired: the 409 is raised
/// in the domain layer, under the lock, and survives the mapping out to HTTP.
#[tokio::test]
async fn update_returns_409_when_demoting_the_only_admin() {
    let organization_id = Id::new_v4();
    let target_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(
        mock_through_extractor(&user, &role, organization_id)
            .append_query_results([vec![test_organization(organization_id)]])
            .append_query_results([vec![requester()]])
            .append_query_results([vec![test_role(
                target_id,
                Some(organization_id),
                users::Role::Admin,
            )]])
            .append_query_results([vec![test_role(
                target_id,
                Some(organization_id),
                users::Role::Admin,
            )]])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "PUT",
        organization_id,
        target_id,
        Body::from(r#"{"role":"User"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

/// A member who holds no role in this organization is a 404, not a silent creation:
/// `POST` creates memberships, `PUT` changes them.
#[tokio::test]
async fn update_returns_404_for_a_user_who_holds_no_role_here() {
    let organization_id = Id::new_v4();
    let user = requester();
    let role = test_role(user.id, Some(organization_id), users::Role::Admin);

    let db = Arc::new(
        mock_through_extractor(&user, &role, organization_id)
            .append_query_results([vec![test_organization(organization_id)]])
            .append_query_results([vec![requester()]])
            .append_query_results([Vec::<user_roles::Model>::new()])
            .into_connection(),
    );

    let app = build_app(db);
    let cookie = login_cookie(&app).await;

    let response = role_request(
        &app,
        &cookie,
        "PUT",
        organization_id,
        Id::new_v4(),
        Body::from(r#"{"role":"Admin"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
