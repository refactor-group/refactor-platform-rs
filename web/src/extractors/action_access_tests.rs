//! The action routes through the router, sign-in and access checks, against a mock database:
//! the assignees and status a participant's read or write returns.

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
use domain::{actions, Id};
use sea_orm::{DatabaseConnection, MockDatabase, MockExecResult, Value};
use serde_json::json;

use crate::controller::action_controller::{create, delete, read, update, update_status};
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, assert_status_written, json_body, Caller,
};
use crate::AppState;

/// The shared relationship and session, with one action in the session.
struct Fixture {
    base: support::Fixture,
    action: actions::Model,
}

impl Deref for Fixture {
    type Target = support::Fixture;

    fn deref(&self) -> &support::Fixture {
        &self.base
    }
}

fn fixture() -> Fixture {
    let now = Utc::now();
    let base = support::fixture();
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
    Fixture { base, action }
}

impl Fixture {
    /// `caller` signed in, with the action and its session queued for `ActionAccess`.
    fn reaching_action(&self, caller: Caller) -> MockDatabase {
        self.signed_in(caller)
            .append_query_results([vec![self.action.clone()]])
            .append_query_results([vec![(self.session.clone(), self.relationship.clone())]])
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
    let w = fixture();
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
async fn act_28_coach_creates_an_action_assigned_to_the_coachee() {
    let w = fixture();
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
async fn m3_09_update_keeps_a_removed_coachee_already_assigned() {
    let w = fixture();
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
async fn act_30_coach_updates_the_action_status() {
    let w = fixture();
    let on_hold = actions::Model {
        status: domain::status::Status::OnHold,
        ..w.action.clone()
    };
    let db = Arc::new(
        w.reaching_action(Caller::Coach)
            // The locked read, then the UPDATE ... RETURNING.
            .append_query_results([vec![w.action.clone()]])
            .append_query_results([vec![on_hold]])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coach.email,
        "PUT",
        &format!("/actions/{}/status?value=on_hold", w.action.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "ACT-30");
    let body = json_body(response).await;
    assert_eq!(body["data"]["id"], json!(w.action.id), "ACT-30");
    assert_eq!(body["data"]["status"], json!("OnHold"), "ACT-30");
    assert_status_written("ACT-30", &db, "on_hold");
}

#[tokio::test]
async fn act_30b_unknown_status_value_is_refused() {
    let w = fixture();
    assert_refused(
        "ACT-30b",
        w.reaching_action(Caller::Coach),
        &w.coach.email,
        "PUT",
        &format!("/actions/{}/status?value=finished", w.action.id),
        None,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
}
