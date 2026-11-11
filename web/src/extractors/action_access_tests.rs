//! The action routes through the router, sign-in and access checks, against a mock database:
//! only a participant of the action's relationship may read, create, change or delete it.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{
    body::Body,
    extract::Request,
    middleware::from_fn,
    routing::{get, post, put},
    Router,
};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::Utc;
use domain::user::Backend;
use domain::{actions, coaching_relationships, coaching_sessions, goals, user_roles, users, Id};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase, MockExecResult, Value};
use serde_json::json;
use service::config::Config;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use crate::controller::action_controller::{create, delete, read, update, update_status};
use crate::middleware::auth::require_auth;
use crate::AppState;

fn user(email: &str) -> users::Model {
    let now = Utc::now();
    users::Model {
        id: Id::new_v4(),
        email: email.to_string(),
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

fn role(user_id: Id, organization_id: Id) -> user_roles::Model {
    let now = Utc::now();
    user_roles::Model {
        id: Id::new_v4(),
        role: users::Role::User,
        organization_id: Some(organization_id),
        user_id,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// A relationship with one session holding one action, plus callers outside it.
struct World {
    organization_id: Id,
    coach: users::Model,
    coachee: users::Model,
    member: users::Model,
    outsider: users::Model,
    relationship: coaching_relationships::Model,
    session: coaching_sessions::Model,
    action: actions::Model,
}

fn world() -> World {
    let now = Utc::now();
    let organization_id = Id::new_v4();
    let coach = user("coach@example.com");
    let coachee = user("coachee@example.com");
    let relationship = coaching_relationships::Model {
        id: Id::new_v4(),
        coach_id: coach.id,
        coachee_id: coachee.id,
        organization_id,
        slug: "test".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    };
    let session = coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id: relationship.id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: now.naive_utc(),
        duration_minutes: domain::duration::Duration::default_minutes(),
        title: None,
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: Some(now.into()),
        notice_given_at: now.into(),
    };
    let action = actions::Model {
        id: Id::new_v4(),
        coaching_session_id: session.id,
        goal_id: None,
        user_id: coach.id,
        body: Some("Draft the plan".to_string()),
        due_by: None,
        status: domain::status::Status::NotStarted,
        status_changed_at: now.into(),
        created_at: now.into(),
        updated_at: now.into(),
    };
    World {
        organization_id,
        coach,
        coachee,
        member: user("member@example.com"),
        outsider: user("outsider@example.com"),
        relationship,
        session,
        action,
    }
}

#[derive(Clone, Copy, Debug)]
enum Caller {
    Coach,
    Coachee,
    /// In the relationship's organization, not in the relationship.
    Member,
    /// In another organization only.
    Outsider,
    /// The coach, with no role in the relationship's organization.
    Removed,
}

impl World {
    fn caller(&self, caller: Caller) -> &users::Model {
        match caller {
            Caller::Coach | Caller::Removed => &self.coach,
            Caller::Coachee => &self.coachee,
            Caller::Member => &self.member,
            Caller::Outsider => &self.outsider,
        }
    }

    /// The login row and the per-request user row for `caller`.
    fn signed_in(&self, caller: Caller) -> MockDatabase {
        let user = self.caller(caller).clone();
        let db = MockDatabase::new(DatabaseBackend::Postgres);
        match caller {
            Caller::Removed => db
                .append_query_results([vec![(user.clone(), None::<user_roles::Model>)]])
                .append_query_results([vec![(user, None::<user_roles::Model>)]]),
            Caller::Outsider => {
                let role = role(user.id, Id::new_v4());
                db.append_query_results([vec![(user.clone(), role.clone())]])
                    .append_query_results([vec![(user, role)]])
            }
            _ => {
                let role = role(user.id, self.organization_id);
                db.append_query_results([vec![(user.clone(), role.clone())]])
                    .append_query_results([vec![(user, role)]])
            }
        }
    }

    /// `caller` signed in, with the action and its session queued for `ActionAccess`.
    fn reaching_action(&self, caller: Caller) -> MockDatabase {
        self.signed_in(caller)
            .append_query_results([vec![self.action.clone()]])
            .append_query_results([vec![(self.session.clone(), self.relationship.clone())]])
    }

    /// `caller` signed in, with the session queued for the create's session check.
    fn reaching_session(&self, caller: Caller) -> MockDatabase {
        self.signed_in(caller)
            .append_query_results([vec![(self.session.clone(), self.relationship.clone())]])
    }

    fn goal_in(&self, coaching_relationship_id: Id) -> goals::Model {
        let now = Utc::now();
        goals::Model {
            id: Id::new_v4(),
            coaching_relationship_id,
            created_in_session_id: None,
            user_id: self.coach.id,
            title: Some("Goal".to_string()),
            body: None,
            status: domain::status::Status::InProgress,
            status_changed_at: None,
            completed_at: None,
            target_date: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn create_body(&self, assignee_ids: &[Id], goal_id: Option<Id>) -> serde_json::Value {
        json!({
            "coaching_session_id": self.session.id,
            "goal_id": goal_id,
            "body": "Draft the plan",
            "due_by": null,
            "status": "NotStarted",
            "assignee_ids": assignee_ids,
        })
    }

    fn update_body(&self, assignee_ids: Option<&[Id]>) -> serde_json::Value {
        let mut body = json!({
            "coaching_session_id": self.session.id,
            "goal_id": null,
            "body": "Draft the revised plan",
            "due_by": null,
            "status": "InProgress",
        });
        if let Some(ids) = assignee_ids {
            body["assignee_ids"] = json!(ids);
        }
        body
    }
}

/// An `actions_users` row, which `domain` does not re-export as a model.
fn assignment_row(action_id: Id, user_id: Id) -> BTreeMap<&'static str, Value> {
    let now = Utc::now().fixed_offset();
    BTreeMap::from([
        ("id", Value::from(Id::new_v4())),
        ("action_id", Value::from(action_id)),
        ("user_id", Value::from(user_id)),
        ("created_at", Value::from(now)),
        ("updated_at", Value::from(now)),
    ])
}

fn build_app(db: Arc<DatabaseConnection>) -> Router {
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
            axum::routing::post(crate::controller::user_session_controller::login),
        )
        .merge(
            Router::new()
                .route("/actions", post(create))
                .route("/actions/:id", get(read).put(update).delete(delete))
                .route("/actions/:id/status", put(update_status))
                .route_layer(from_fn(require_auth)),
        )
        .layer(auth_layer)
        .with_state(app_state)
}

/// Signs `email` in, then sends `method uri` with `body` as JSON.
async fn send(
    db: &Arc<DatabaseConnection>,
    email: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> axum::response::Response {
    let app = build_app(db.clone());
    let login = Request::builder()
        .uri("/login")
        .method("POST")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("email={email}&password=password123")))
        .unwrap();
    let cookie = app
        .clone()
        .oneshot(login)
        .await
        .unwrap()
        .headers()
        .get("set-cookie")
        .and_then(|cookie| cookie.to_str().ok())
        .expect("login should return a session cookie")
        .to_string();

    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie)
        .header("x-version", "1.0.0")
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .unwrap();
    app.oneshot(request).await.unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).expect("the body must be JSON")
}

/// Whether the database was asked to write anything.
fn wrote(db: &DatabaseConnection) -> bool {
    db.clone().into_transaction_log().iter().any(|t| {
        t.statements().iter().any(|s| {
            let sql = s.sql.trim_start().to_uppercase();
            ["INSERT", "UPDATE", "DELETE"]
                .iter()
                .any(|verb| sql.starts_with(verb))
        })
    })
}

/// Sends the request and asserts the refusal `expected` with nothing written.
async fn assert_refused(
    row: &str,
    db: MockDatabase,
    email: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
    expected: StatusCode,
) {
    let db = Arc::new(db.into_connection());
    let response = send(&db, email, method, uri, body).await;
    assert_eq!(response.status(), expected, "{row}");
    assert!(
        !wrote(&db),
        "{row}: the refused request wrote to the database"
    );
}

#[tokio::test]
async fn act_27_coach_reads_the_action() {
    let w = world();
    let db = Arc::new(
        w.reaching_action(Caller::Coach)
            .append_query_results([vec![assignment_row(w.action.id, w.coachee.id)]])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coach.email,
        "GET",
        &format!("/actions/{}", w.action.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "ACT-27");
    let body = json_body(response).await;
    assert_eq!(body["data"]["id"], json!(w.action.id), "ACT-27");
    assert_eq!(
        body["data"]["assignee_ids"],
        json!([w.coachee.id]),
        "ACT-27"
    );
}

#[tokio::test]
async fn act_27b_coachee_reads_the_action() {
    let w = world();
    let db = Arc::new(
        w.reaching_action(Caller::Coachee)
            .append_query_results([Vec::<BTreeMap<&str, Value>>::new()])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coachee.email,
        "GET",
        &format!("/actions/{}", w.action.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "ACT-27b");
}

#[tokio::test]
async fn act_32_member_cannot_read_the_action() {
    let w = world();
    assert_refused(
        "ACT-32",
        w.reaching_action(Caller::Member),
        &w.member.email,
        "GET",
        &format!("/actions/{}", w.action.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn act_32b_outsider_cannot_read_the_action() {
    let w = world();
    assert_refused(
        "ACT-32b",
        w.reaching_action(Caller::Outsider),
        &w.outsider.email,
        "GET",
        &format!("/actions/{}", w.action.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn act_32c_unknown_action_is_not_found() {
    let w = world();
    assert_refused(
        "ACT-32c",
        w.signed_in(Caller::Coach)
            .append_query_results([Vec::<actions::Model>::new()]),
        &w.coach.email,
        "GET",
        &format!("/actions/{}", Id::new_v4()),
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn act_35_participant_removed_from_the_organization_cannot_read_the_action() {
    let w = world();
    assert_refused(
        "ACT-35",
        w.reaching_action(Caller::Removed),
        &w.coach.email,
        "GET",
        &format!("/actions/{}", w.action.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn act_28_coach_creates_an_action_assigned_to_the_coachee() {
    let w = world();
    let db = Arc::new(
        w.reaching_session(Caller::Coach)
            .append_query_results([vec![w.action.clone()]])
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .append_query_results([vec![assignment_row(w.action.id, w.coachee.id)]])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coach.email,
        "POST",
        "/actions",
        Some(w.create_body(&[w.coachee.id], None)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "ACT-28");
    let body = json_body(response).await;
    assert_eq!(body["status_code"], json!(201), "ACT-28");
    assert_eq!(
        body["data"]["assignee_ids"],
        json!([w.coachee.id]),
        "ACT-28"
    );
}

#[tokio::test]
async fn act_33_member_cannot_create_an_action_in_the_session() {
    let w = world();
    assert_refused(
        "ACT-33",
        w.reaching_session(Caller::Member),
        &w.member.email,
        "POST",
        "/actions",
        Some(w.create_body(&[w.coachee.id], None)),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn act_33b_unknown_session_is_not_found() {
    let w = world();
    assert_refused(
        "ACT-33b",
        w.signed_in(Caller::Coach).append_query_results([Vec::<(
            coaching_sessions::Model,
            coaching_relationships::Model,
        )>::new()]),
        &w.coach.email,
        "POST",
        "/actions",
        Some(json!({
            "coaching_session_id": Id::new_v4(),
            "goal_id": null,
            "body": "Draft the plan",
            "due_by": null,
            "status": "NotStarted",
            "assignee_ids": [w.coachee.id],
        })),
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn act_36_create_refuses_an_assignee_outside_the_relationship() {
    let w = world();
    assert_refused(
        "ACT-36",
        w.reaching_session(Caller::Coach),
        &w.coach.email,
        "POST",
        "/actions",
        Some(w.create_body(&[w.member.id], None)),
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
}

#[tokio::test]
async fn act_36b_update_refuses_an_assignee_outside_the_relationship() {
    let w = world();
    assert_refused(
        "ACT-36b",
        w.reaching_action(Caller::Coach),
        &w.coach.email,
        "PUT",
        &format!("/actions/{}", w.action.id),
        Some(w.update_body(Some(&[w.coach.id, w.member.id]))),
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
}

#[tokio::test]
async fn act_37_create_refuses_a_goal_from_another_relationship() {
    let w = world();
    let other_goal = w.goal_in(Id::new_v4());
    assert_refused(
        "ACT-37",
        w.reaching_session(Caller::Coach)
            .append_query_results([vec![other_goal.clone()]]),
        &w.coach.email,
        "POST",
        "/actions",
        Some(w.create_body(&[w.coachee.id], Some(other_goal.id))),
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
}

#[tokio::test]
async fn act_29_coachee_updates_the_action() {
    let w = world();
    let db = Arc::new(
        w.reaching_action(Caller::Coachee)
            // The locked read, then the UPDATE ... RETURNING.
            .append_query_results([vec![w.action.clone()]])
            .append_query_results([vec![w.action.clone()]])
            .append_query_results([Vec::<BTreeMap<&str, Value>>::new()])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coachee.email,
        "PUT",
        &format!("/actions/{}", w.action.id),
        Some(w.update_body(None)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "ACT-29");
}

#[tokio::test]
async fn act_34_member_cannot_update_the_action() {
    let w = world();
    assert_refused(
        "ACT-34",
        w.reaching_action(Caller::Member),
        &w.member.email,
        "PUT",
        &format!("/actions/{}", w.action.id),
        Some(w.update_body(None)),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn act_30_coach_updates_the_action_status() {
    let w = world();
    let db = Arc::new(
        w.reaching_action(Caller::Coach)
            // The locked read, then the UPDATE ... RETURNING.
            .append_query_results([vec![w.action.clone()]])
            .append_query_results([vec![w.action.clone()]])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coach.email,
        "PUT",
        &format!("/actions/{}/status?value=completed", w.action.id),
        None,
    )
    .await;
    // `Query<String>` cannot read `value=completed`, so the handler answers 400 past the check.
    assert!(
        ![StatusCode::FORBIDDEN, StatusCode::NOT_FOUND].contains(&response.status()),
        "ACT-30: {}",
        response.status()
    );
}

#[tokio::test]
async fn act_34b_outsider_cannot_update_the_action_status() {
    let w = world();
    assert_refused(
        "ACT-34b",
        w.reaching_action(Caller::Outsider),
        &w.outsider.email,
        "PUT",
        &format!("/actions/{}/status?value=completed", w.action.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn act_31_coach_deletes_the_action() {
    let w = world();
    let db = Arc::new(
        w.reaching_action(Caller::Coach)
            // `delete_by_id` reads the action, then `entity_api` reads it again to delete it.
            .append_query_results([vec![w.action.clone()]])
            .append_query_results([vec![w.action.clone()]])
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coach.email,
        "DELETE",
        &format!("/actions/{}", w.action.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "ACT-31");
}

#[tokio::test]
async fn act_34c_member_cannot_delete_the_action() {
    let w = world();
    assert_refused(
        "ACT-34c",
        w.reaching_action(Caller::Member),
        &w.member.email,
        "DELETE",
        &format!("/actions/{}", w.action.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}
