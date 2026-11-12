//! The goal status route through the router, sign-in and its `protect` layer, against a mock
//! database: it writes the requested status.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::{middleware::from_fn_with_state, routing::put, Router};
use chrono::Utc;
use domain::{goals, Id};
use sea_orm::{DatabaseConnection, MockDatabase};
use serde_json::json;

use super::by_id;
use crate::controller::goal_controller::update_status;
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, assert_status_written, json_body, Caller, Fixture,
};
use crate::AppState;

fn status_routes(db: &Arc<DatabaseConnection>) -> Router<AppState> {
    Router::new()
        .route("/goals/:id/status", put(update_status))
        .route_layer(from_fn_with_state(support::app_state(db), by_id))
}

/// An in-progress goal in the shared relationship.
fn goal(w: &Fixture) -> goals::Model {
    let now = Utc::now();
    goals::Model {
        id: Id::new_v4(),
        coaching_relationship_id: w.relationship.id,
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

/// `caller` signed in, with the goal and its relationship queued for `by_id`.
fn reaching_goal(w: &Fixture, caller: Caller, goal: &goals::Model) -> MockDatabase {
    w.signed_in(caller)
        .append_query_results([vec![goal.clone()]])
        .append_query_results([vec![w.relationship.clone()]])
}

#[tokio::test]
async fn goal_04_coach_updates_the_goal_status() {
    let w = support::fixture();
    let goal = goal(&w);
    let on_hold = goals::Model {
        status: domain::status::Status::OnHold,
        ..goal.clone()
    };
    let db = Arc::new(
        reaching_goal(&w, Caller::Coach, &goal)
            // The read, then the UPDATE ... RETURNING.
            .append_query_results([vec![goal.clone()]])
            .append_query_results([vec![on_hold]])
            .into_connection(),
    );
    let response = support::send(
        status_routes(&db),
        &db,
        &w.coach.email,
        "PUT",
        &format!("/goals/{}/status?value=on_hold", goal.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "GOAL-04");
    let body = json_body(response).await;
    assert_eq!(body["data"]["id"], json!(goal.id), "GOAL-04");
    assert_status_written("GOAL-04", &db, "on_hold");
}

#[tokio::test]
async fn goal_04b_unknown_status_value_is_refused() {
    let w = support::fixture();
    let goal = goal(&w);
    let db = Arc::new(reaching_goal(&w, Caller::Coach, &goal).into_connection());
    let response = support::send(
        status_routes(&db),
        &db,
        &w.coach.email,
        "PUT",
        &format!("/goals/{}/status?value=finished", goal.id),
        None,
    )
    .await;
    assert_refusal("GOAL-04b", &db, response, StatusCode::UNPROCESSABLE_ENTITY);
}
