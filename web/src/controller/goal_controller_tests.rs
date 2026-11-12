//! `POST /goals` through the router, sign-in and access check, against a mock database: a
//! coach creates a goal from a session of the relationship.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{routing::post, Router};
use chrono::Utc;
use domain::{coaching_sessions_goals, goals, Id};
use sea_orm::{MockDatabase, Value};
use serde_json::json;

use super::create;
use crate::extractors::relationship_access_test_support::{
    self as support, json_body, Caller, Fixture,
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

fn create_body_from_session(coaching_relationship_id: Id, session_id: Id) -> serde_json::Value {
    let mut body = create_body(coaching_relationship_id);
    body["created_in_session_id"] = json!(session_id);
    body
}

fn goal_in(w: &Fixture) -> goals::Model {
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
fn members(w: &Fixture) -> Vec<BTreeMap<String, Value>> {
    [w.coach.id, w.coachee.id]
        .into_iter()
        .map(|user_id| BTreeMap::from([("user_id".to_string(), user_id.into())]))
        .collect()
}

/// `caller` signed in, with R1 queued for the relationship check.
fn reaching_relationship(w: &Fixture, caller: Caller) -> MockDatabase {
    w.signed_in(caller)
        .append_query_results([vec![w.relationship.clone()]])
}

#[tokio::test]
async fn goal_28c_coach_creates_a_goal_from_a_session_of_the_relationship() {
    let w = support::fixture();
    let goal = goals::Model {
        created_in_session_id: Some(w.session.id),
        status: domain::status::Status::InProgress,
        ..goal_in(&w)
    };
    let now = Utc::now();
    let link = coaching_sessions_goals::Model {
        id: Id::new_v4(),
        coaching_session_id: w.session.id,
        goal_id: goal.id,
        created_at: now.into(),
        updated_at: now.into(),
    };
    let db = Arc::new(
        reaching_relationship(&w, Caller::Coach)
            .append_query_results([vec![w.session.clone()]])
            .append_query_results([vec![goal.clone()]])
            .append_query_results([vec![goal.clone()]])
            .append_query_results([Vec::<coaching_sessions_goals::Model>::new()])
            .append_query_results([vec![link]])
            .append_query_results([vec![w.relationship.clone()]])
            .append_query_results([members(&w)])
            .into_connection(),
    );
    let response = support::send(
        routes(),
        &db,
        &w.coach.email,
        "POST",
        "/goals",
        Some(create_body_from_session(w.relationship.id, w.session.id)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "GOAL-28c");
    let body = json_body(response).await;
    assert_eq!(body["status_code"], json!(201), "GOAL-28c");
    assert_eq!(body["data"]["id"], json!(goal.id), "GOAL-28c");
}
