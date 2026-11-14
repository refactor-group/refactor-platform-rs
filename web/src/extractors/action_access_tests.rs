//! The action status route through the router, sign-in and access check, against a mock
//! database: an unknown status value is refused with nothing written.

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
use sea_orm::{DatabaseConnection, DbErr, MockDatabase};

use crate::controller::action_controller::{create, delete, read, update, update_status};
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, Caller,
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

#[tokio::test]
async fn a_failed_action_lookup_is_a_server_error() {
    let w = fixture();
    assert_refused(
        "failed action lookup",
        w.signed_in(Caller::Coach)
            .append_query_errors([DbErr::Custom("connection lost".to_string())]),
        &w.coach.email,
        "GET",
        &format!("/actions/{}", w.action.id),
        None,
        StatusCode::SERVICE_UNAVAILABLE,
    )
    .await;
}
