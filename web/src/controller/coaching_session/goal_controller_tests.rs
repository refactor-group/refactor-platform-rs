//! The session goal link routes through the router, sign-in and access check, against a mock
//! database: only a participant may link or unlink, and only a goal of the session's
//! relationship may be linked.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{
    routing::{delete as delete_route, post},
    Router,
};
use chrono::Utc;
use domain::{coaching_sessions_goals, goals, Id};
use sea_orm::{MockDatabase, MockExecResult, Value};
use serde_json::json;

use super::{create, delete};
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, json_body, Caller, World,
};
use crate::AppState;

fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/coaching_sessions/:coaching_session_id/goals",
            post(create),
        )
        .route(
            "/coaching_sessions/:coaching_session_id/goals/:id",
            delete_route(delete),
        )
}

fn goal_in(w: &World, coaching_relationship_id: Id) -> goals::Model {
    let now = Utc::now();
    goals::Model {
        id: Id::new_v4(),
        coaching_relationship_id,
        created_in_session_id: None,
        user_id: w.coach.id,
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

fn link(w: &World, goal_id: Id) -> coaching_sessions_goals::Model {
    let now = Utc::now().fixed_offset();
    coaching_sessions_goals::Model {
        id: Id::new_v4(),
        coaching_session_id: w.session.id,
        goal_id,
        created_at: now,
        updated_at: now,
    }
}

/// The participants who still belong to the relationship's organization.
fn members(w: &World) -> Vec<BTreeMap<String, Value>> {
    [w.coach.id, w.coachee.id]
        .into_iter()
        .map(|user_id| BTreeMap::from([("user_id".to_string(), user_id.into())]))
        .collect()
}

fn goals_uri(w: &World) -> String {
    format!("/coaching_sessions/{}/goals", w.session.id)
}

fn goal_uri(w: &World, goal_id: Id) -> String {
    format!("/coaching_sessions/{}/goals/{goal_id}", w.session.id)
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
    let response = support::send(routes(), &db, email, method, uri, body).await;
    assert_refusal(row, &db, response, expected);
}

#[tokio::test]
async fn goal_13_coach_links_a_goal_of_the_relationship() {
    let w = support::world();
    let goal = goal_in(&w, w.relationship.id);
    let link = link(&w, goal.id);
    let db = Arc::new(
        w.reaching_session(Caller::Coach)
            .append_query_results([vec![goal.clone()]])
            // The domain's own session lookup, then the link itself.
            .append_query_results([vec![(w.session.clone(), w.relationship.clone())]])
            .append_query_results([members(&w)])
            .append_query_results([vec![goal.clone()]])
            .append_query_results([Vec::<coaching_sessions_goals::Model>::new()])
            .append_query_results([vec![link.clone()]])
            .into_connection(),
    );
    let response = support::send(
        routes(),
        &db,
        &w.coach.email,
        "POST",
        &goals_uri(&w),
        Some(json!({ "goal_id": goal.id })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "GOAL-13");
    let body = json_body(response).await;
    assert_eq!(body["status_code"], json!(201), "GOAL-13");
    assert_eq!(body["data"]["goal_id"], json!(goal.id), "GOAL-13");
}

#[tokio::test]
async fn goal_26_member_cannot_link_a_goal_to_the_session() {
    let w = support::world();
    let goal = goal_in(&w, w.relationship.id);
    assert_refused(
        "GOAL-26",
        w.reaching_session(Caller::Member),
        &w.member.email,
        "POST",
        &goals_uri(&w),
        Some(json!({ "goal_id": goal.id })),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn goal_27_link_refuses_a_goal_from_another_relationship() {
    let w = support::world();
    let other_goal = goal_in(&w, Id::new_v4());
    assert_refused(
        "GOAL-27",
        w.reaching_session(Caller::Coach)
            .append_query_results([vec![other_goal.clone()]]),
        &w.coach.email,
        "POST",
        &goals_uri(&w),
        Some(json!({ "goal_id": other_goal.id })),
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await;
}

#[tokio::test]
async fn goal_27b_link_of_an_unknown_goal_is_not_found() {
    let w = support::world();
    assert_refused(
        "GOAL-27b",
        w.reaching_session(Caller::Coach)
            .append_query_results([Vec::<goals::Model>::new()]),
        &w.coach.email,
        "POST",
        &goals_uri(&w),
        Some(json!({ "goal_id": Id::new_v4() })),
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn goal_14_coachee_unlinks_a_goal_from_the_session() {
    let w = support::world();
    let goal = goal_in(&w, w.relationship.id);
    let db = Arc::new(
        w.reaching_session(Caller::Coachee)
            .append_query_results([vec![(link(&w, goal.id), w.relationship.clone())]])
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results([members(&w)])
            .into_connection(),
    );
    let response = support::send(
        routes(),
        &db,
        &w.coachee.email,
        "DELETE",
        &goal_uri(&w, goal.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "GOAL-14");
}

#[tokio::test]
async fn goal_26b_outsider_cannot_unlink_a_goal_from_the_session() {
    let w = support::world();
    let goal = goal_in(&w, w.relationship.id);
    assert_refused(
        "GOAL-26b",
        w.reaching_session(Caller::Outsider),
        &w.outsider.email,
        "DELETE",
        &goal_uri(&w, goal.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}
