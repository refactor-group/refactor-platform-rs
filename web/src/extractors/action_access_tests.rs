//! The action routes through the router, sign-in and access checks, against a mock database:
//! only a participant of the action's relationship may read, create, change or delete it.

use std::collections::BTreeMap;
use std::ops::Deref;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{
    response::Response,
    routing::{get, post, put},
    Router,
};
use chrono::Utc;
use domain::{actions, coaching_relationships, coaching_sessions, goals, Id};
use sea_orm::{DatabaseConnection, MockDatabase, MockExecResult, Value};
use serde_json::json;

use crate::controller::action_controller::{create, delete, read, update, update_status};
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, json_body, Caller,
};
use crate::AppState;

/// The shared relationship and session, with one action in the session.
struct World {
    base: support::World,
    action: actions::Model,
}

impl Deref for World {
    type Target = support::World;

    fn deref(&self) -> &support::World {
        &self.base
    }
}

fn world() -> World {
    let now = Utc::now();
    let base = support::world();
    let action = actions::Model {
        id: Id::new_v4(),
        coaching_session_id: base.session.id,
        goal_id: None,
        user_id: base.coach.id,
        body: Some("Draft the plan".to_string()),
        due_by: None,
        status: domain::status::Status::NotStarted,
        status_changed_at: now.into(),
        created_at: now.into(),
        updated_at: now.into(),
    };
    World { base, action }
}

impl World {
    /// `caller` signed in, with the action and its session queued for `ActionAccess`.
    fn reaching_action(&self, caller: Caller) -> MockDatabase {
        self.signed_in(caller)
            .append_query_results([vec![self.action.clone()]])
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

fn routes() -> Router<AppState> {
    Router::new()
        .route("/actions", post(create))
        .route("/actions/:id", get(read).put(update).delete(delete))
        .route("/actions/:id/status", put(update_status))
}

/// Signs `email` in, then sends `method uri` with `body` as JSON.
async fn send(
    db: &Arc<DatabaseConnection>,
    email: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> Response {
    support::send(routes(), db, email, method, uri, body).await
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
    assert_refusal(row, &db, response, expected);
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
            .append_query_results([support::members(&[w.coachee.id])])
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
        w.reaching_action(Caller::Coach)
            .append_query_results([Vec::<BTreeMap<&str, Value>>::new()]),
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
            .append_query_results([support::members(&[w.coachee.id])])
            .append_query_results([vec![other_goal.clone()]]),
        &w.coach.email,
        "POST",
        "/actions",
        Some(w.create_body(&[w.coachee.id], Some(other_goal.id))),
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
}

/// Sends the request and asserts the 409 refusing the removed coachee as an assignee,
/// with nothing written.
async fn assert_assignee_refused(
    row: &str,
    w: &World,
    db: MockDatabase,
    method: &str,
    uri: &str,
    body: serde_json::Value,
) {
    let db = Arc::new(db.into_connection());
    let response = send(&db, &w.coach.email, method, uri, Some(body)).await;
    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(status, StatusCode::CONFLICT, "{row}: {body}");
    support::assert_not_a_member(
        row,
        &body,
        "assignee_not_a_member",
        w.coachee.id,
        w.organization_id,
    );
    support::assert_nothing_written(row, &db);
}

#[tokio::test]
async fn m3_07_create_refuses_assigning_a_coachee_no_longer_in_the_organization() {
    let w = world();
    assert_assignee_refused(
        "M3-07",
        &w,
        w.reaching_session(Caller::Coach)
            .append_query_results([support::members(&[w.coach.id])]),
        "POST",
        "/actions",
        w.create_body(&[w.coach.id, w.coachee.id], None),
    )
    .await;
}

#[tokio::test]
async fn m3_08_update_refuses_adding_a_coachee_no_longer_in_the_organization() {
    let w = world();
    assert_assignee_refused(
        "M3-08",
        &w,
        w.reaching_action(Caller::Coach)
            .append_query_results([vec![assignment_row(w.action.id, w.coach.id)]])
            .append_query_results([Vec::<BTreeMap<String, Value>>::new()]),
        "PUT",
        &format!("/actions/{}", w.action.id),
        w.update_body(Some(&[w.coach.id, w.coachee.id])),
    )
    .await;
}

#[tokio::test]
async fn m3_09_update_keeps_a_removed_coachee_already_assigned() {
    let w = world();
    let db = Arc::new(
        w.reaching_action(Caller::Coach)
            .append_query_results([vec![assignment_row(w.action.id, w.coachee.id)]])
            // The locked read, then the UPDATE ... RETURNING.
            .append_query_results([vec![w.action.clone()]])
            .append_query_results([vec![w.action.clone()]])
            // The assignees replaced as they were.
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results([vec![assignment_row(w.action.id, w.coachee.id)]])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coach.email,
        "PUT",
        &format!("/actions/{}", w.action.id),
        Some(w.update_body(Some(&[w.coachee.id]))),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "M3-09");
    let body = json_body(response).await;
    assert_eq!(body["data"]["assignee_ids"], json!([w.coachee.id]), "M3-09");
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
