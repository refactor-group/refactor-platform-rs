//! `POST /coaching_session_series` through the router, sign-in and access check, against a
//! mock database and a mock Resend: only the relationship's coach, still a member of its
//! organization, may create a series in it.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::{routing::post, Router};
use chrono::Utc;
use domain::{coaching_session_series, coaching_sessions, Id};
use mockito::Server;
use sea_orm::MockDatabase;
use serde_json::json;

use super::create;
use crate::controller::coaching_session_controller::coaching_session_create_tests::{
    config, no_outbound_posts, reaching_relationship, tomorrow,
};
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, json_body, Caller, World,
};
use crate::AppState;

fn routes() -> Router<AppState> {
    Router::new().route("/coaching_session_series", post(create))
}

fn create_body(w: &World) -> serde_json::Value {
    json!({
        "coaching_relationship_id": w.relationship.id,
        "start_at": tomorrow(),
        "recurrence": { "frequency": "weekly", "interval": 1, "count": 2 },
        "duration_minutes": 60,
    })
}

/// Sends `POST /coaching_session_series` for R1 and asserts the refusal `expected` with
/// nothing written and nothing sent.
async fn assert_refused(row: &str, w: &World, caller: Caller, email: &str, expected: StatusCode) {
    let mut server = Server::new_async().await;
    let outbound = no_outbound_posts(&mut server).await;
    let db = Arc::new(reaching_relationship(w, caller).into_connection());
    let response = support::send_with(
        config(&server),
        routes(),
        &db,
        email,
        "POST",
        "/coaching_session_series",
        Some(create_body(w)),
    )
    .await;
    assert_refusal(row, &db, response, expected);
    outbound.assert_async().await;
}

#[tokio::test]
async fn ser_01_coach_creates_a_series_in_the_relationship() {
    let w = support::world();
    let server = Server::new_async().await;
    let now = Utc::now();
    let series = coaching_session_series::Model {
        id: Id::new_v4(),
        coaching_relationship_id: w.relationship.id,
        rule: json!({}),
        ical_sequence: 0,
        created_by_user_id: w.coach.id,
        created_at: now.into(),
        updated_at: now.into(),
    };
    let sessions: Vec<coaching_sessions::Model> = [0, 1]
        .into_iter()
        .map(|week| {
            let date = tomorrow() + chrono::Duration::weeks(week);
            coaching_sessions::Model {
                id: Id::new_v4(),
                coaching_session_series_id: Some(series.id),
                ical_recurrence_id: Some(date),
                date,
                ..w.session.clone()
            }
        })
        .collect();
    let db: MockDatabase = reaching_relationship(&w, Caller::Coach)
        .append_query_results([vec![w.relationship.clone()]])
        .append_query_results([vec![w.coach.clone()]])
        .append_query_results([vec![series.clone()]])
        .append_query_results([sessions]);
    let db = Arc::new(db.into_connection());
    let response = support::send_with(
        config(&server),
        routes(),
        &db,
        &w.coach.email,
        "POST",
        "/coaching_session_series",
        Some(create_body(&w)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "SER-01");
    let body = json_body(response).await;
    assert_eq!(body["status_code"], json!(201), "SER-01");
    assert_eq!(body["data"]["series"]["id"], json!(series.id), "SER-01");
}

#[tokio::test]
async fn ser_02_coachee_cannot_create_a_series() {
    let w = support::world();
    assert_refused(
        "SER-02",
        &w,
        Caller::Coachee,
        &w.coachee.email,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn ser_03_member_cannot_create_a_series() {
    let w = support::world();
    assert_refused(
        "SER-03",
        &w,
        Caller::Member,
        &w.member.email,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn ser_16_removed_coach_cannot_create_a_series() {
    let w = support::world();
    assert_refused(
        "SER-16",
        &w,
        Caller::Removed,
        &w.coach.email,
        StatusCode::FORBIDDEN,
    )
    .await;
}
