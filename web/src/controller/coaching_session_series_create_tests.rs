//! `POST /coaching_session_series` through the router, sign-in and access check, against a
//! mock database and a mock Resend: a series with a coachee no longer in the organization is
//! refused with nothing written and nothing sent.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::{routing::post, Router};
use mockito::Server;
use serde_json::json;

use super::create;
use crate::controller::coaching_session_controller::coaching_session_create_tests::{
    config, no_outbound_posts, reaching_relationship, tomorrow,
};
use crate::extractors::relationship_access_test_support::{
    self as support, json_body, Caller, World,
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

#[tokio::test]
async fn m3_02_coach_cannot_schedule_a_series_with_a_coachee_no_longer_in_the_organization() {
    let w = support::world();
    let mut server = Server::new_async().await;
    let outbound = no_outbound_posts(&mut server).await;
    let db = Arc::new(
        reaching_relationship(&w, Caller::Coach)
            .append_query_results([vec![w.relationship.clone()]])
            .append_query_results([support::members(&[w.coach.id])])
            .into_connection(),
    );
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
    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(status, StatusCode::CONFLICT, "M3-02: {body}");
    support::assert_not_a_member(
        "M3-02",
        &body,
        "coachee_not_a_member",
        w.coachee.id,
        w.organization_id,
    );
    support::assert_nothing_written("M3-02", &db);
    outbound.assert_async().await;
}
