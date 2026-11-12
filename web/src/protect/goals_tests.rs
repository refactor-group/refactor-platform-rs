//! Goal routes through the router, sign-in and their `protect` layer, against a mock database:
//! every requested session must belong to a relationship of the caller, and a goal's status
//! route writes the requested status.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::{
    middleware::from_fn_with_state,
    routing::{get, put},
    Router,
};
use chrono::Utc;
use domain::{coaching_relationships, coaching_sessions, coaching_sessions_goals, goals, Id};
use sea_orm::{DatabaseConnection, MockDatabase};
use serde_json::json;

use super::{batch_by_session, by_id};
use crate::controller::coaching_session::goal_controller::batch_index;
use crate::controller::goal_controller::update_status;
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, assert_status_written, json_body, Caller, World,
};
use crate::AppState;

fn routes(db: &Arc<DatabaseConnection>) -> Router<AppState> {
    Router::new()
        .route("/coaching_sessions/goals", get(batch_index))
        .route_layer(from_fn_with_state(support::app_state(db), batch_by_session))
}

fn status_routes(db: &Arc<DatabaseConnection>) -> Router<AppState> {
    Router::new()
        .route("/goals/:id/status", put(update_status))
        .route_layer(from_fn_with_state(support::app_state(db), by_id))
}

/// An in-progress goal in the shared relationship.
fn goal(w: &World) -> goals::Model {
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
fn reaching_goal(w: &World, caller: Caller, goal: &goals::Model) -> MockDatabase {
    w.signed_in(caller)
        .append_query_results([vec![goal.clone()]])
        .append_query_results([vec![w.relationship.clone()]])
}

/// R2, a relationship in the same organization without the coach or coachee, and S2 in it.
fn other_session(w: &World) -> (coaching_sessions::Model, coaching_relationships::Model) {
    let now = Utc::now();
    let relationship = coaching_relationships::Model {
        id: Id::new_v4(),
        coach_id: Id::new_v4(),
        coachee_id: Id::new_v4(),
        organization_id: w.organization_id,
        slug: "other".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    };
    let session = coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id: relationship.id,
        ..w.session.clone()
    };
    (session, relationship)
}

fn batch_uri(session_ids: &[Id]) -> String {
    let ids: Vec<String> = session_ids.iter().map(Id::to_string).collect();
    format!(
        "/coaching_sessions/goals?coaching_session_ids={}",
        ids.join(",")
    )
}

/// Sends the batch read and asserts the refusal `expected` with nothing written.
async fn assert_refused(row: &str, db: MockDatabase, email: &str, uri: &str, expected: StatusCode) {
    let db = Arc::new(db.into_connection());
    let response = support::send(routes(&db), &db, email, "GET", uri, None).await;
    assert_refusal(row, &db, response, expected);
}

#[tokio::test]
async fn goal_17_coach_cannot_batch_read_a_session_of_another_relationship() {
    let w = support::world();
    let other = other_session(&w);
    assert_refused(
        "GOAL-17",
        w.reaching_session(Caller::Coach)
            .append_query_results([vec![other.clone()]]),
        &w.coach.email,
        &batch_uri(&[w.session.id, other.0.id]),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn goal_17b_coach_batch_reads_their_own_session() {
    let w = support::world();
    let db = Arc::new(
        w.reaching_session(Caller::Coach)
            .append_query_results([Vec::<coaching_sessions_goals::Model>::new()])
            .into_connection(),
    );
    let response = support::send(
        routes(&db),
        &db,
        &w.coach.email,
        "GET",
        &batch_uri(&[w.session.id]),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "GOAL-17b");
}

#[tokio::test]
async fn goal_17c_order_of_the_session_ids_does_not_matter() {
    let w = support::world();
    let other = other_session(&w);
    assert_refused(
        "GOAL-17c",
        w.signed_in(Caller::Coach)
            .append_query_results([vec![other.clone()]])
            .append_query_results([vec![(w.session.clone(), w.relationship.clone())]]),
        &w.coach.email,
        &batch_uri(&[other.0.id, w.session.id]),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn goal_04_coach_updates_the_goal_status() {
    let w = support::world();
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
    let w = support::world();
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
