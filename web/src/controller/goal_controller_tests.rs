//! `POST /goals` through the router, sign-in and access check, against a mock database:
//! only a participant of the body's relationship may create a goal in it.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{routing::post, Router};
use chrono::Utc;
use domain::{coaching_relationships, goals, Id};
use sea_orm::{MockDatabase, Value};
use serde_json::json;

use super::create;
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, json_body, Caller, World,
};
use crate::AppState;

fn routes() -> Router<AppState> {
    Router::new().route("/goals", post(create))
}

fn create_body(coaching_relationship_id: Id) -> serde_json::Value {
    json!({
        "coaching_relationship_id": coaching_relationship_id,
        "created_in_session_id": null,
        "title": "Ship the plan",
        "body": null,
        "status": "NotStarted",
        "target_date": null,
    })
}

fn goal_in(w: &World) -> goals::Model {
    let now = Utc::now();
    goals::Model {
        id: Id::new_v4(),
        coaching_relationship_id: w.relationship.id,
        created_in_session_id: None,
        user_id: w.coachee.id,
        title: Some("Ship the plan".to_string()),
        body: None,
        status: domain::status::Status::NotStarted,
        status_changed_at: Some(now.into()),
        completed_at: None,
        target_date: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// The participants who still belong to the relationship's organization.
fn members(w: &World) -> Vec<BTreeMap<String, Value>> {
    [w.coach.id, w.coachee.id]
        .into_iter()
        .map(|user_id| BTreeMap::from([("user_id".to_string(), user_id.into())]))
        .collect()
}

/// `caller` signed in, with R1 queued for the relationship check.
fn reaching_relationship(w: &World, caller: Caller) -> MockDatabase {
    w.signed_in(caller)
        .append_query_results([vec![w.relationship.clone()]])
}

/// Sends `POST /goals` and asserts the refusal `expected` with nothing written.
async fn assert_refused(
    row: &str,
    db: MockDatabase,
    email: &str,
    body: serde_json::Value,
    expected: StatusCode,
) {
    let db = Arc::new(db.into_connection());
    let response = support::send(routes(), &db, email, "POST", "/goals", Some(body)).await;
    assert_refusal(row, &db, response, expected);
}

#[tokio::test]
async fn goal_10_coachee_creates_a_goal_in_the_relationship() {
    let w = support::world();
    let goal = goal_in(&w);
    let db = Arc::new(
        reaching_relationship(&w, Caller::Coachee)
            .append_query_results([vec![goal.clone()]])
            .append_query_results([vec![w.relationship.clone()]])
            .append_query_results([members(&w)])
            .into_connection(),
    );
    let response = support::send(
        routes(),
        &db,
        &w.coachee.email,
        "POST",
        "/goals",
        Some(create_body(w.relationship.id)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "GOAL-10");
    let body = json_body(response).await;
    assert_eq!(body["status_code"], json!(201), "GOAL-10");
    assert_eq!(body["data"]["id"], json!(goal.id), "GOAL-10");
}

#[tokio::test]
async fn goal_25_member_cannot_create_a_goal_in_the_relationship() {
    let w = support::world();
    assert_refused(
        "GOAL-25",
        reaching_relationship(&w, Caller::Member),
        &w.member.email,
        create_body(w.relationship.id),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn goal_25b_outsider_cannot_create_a_goal_in_the_relationship() {
    let w = support::world();
    assert_refused(
        "GOAL-25b",
        reaching_relationship(&w, Caller::Outsider),
        &w.outsider.email,
        create_body(w.relationship.id),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn goal_25c_unknown_relationship_is_not_found() {
    let w = support::world();
    assert_refused(
        "GOAL-25c",
        w.signed_in(Caller::Coach)
            .append_query_results([Vec::<coaching_relationships::Model>::new()]),
        &w.coach.email,
        create_body(Id::new_v4()),
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn goal_25d_participant_removed_from_the_organization_cannot_create_a_goal() {
    let w = support::world();
    assert_refused(
        "GOAL-25d",
        reaching_relationship(&w, Caller::Removed),
        &w.coach.email,
        create_body(w.relationship.id),
        StatusCode::FORBIDDEN,
    )
    .await;
}
