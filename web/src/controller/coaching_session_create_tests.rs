//! `POST /coaching_sessions` through the router, sign-in and access check, against a mock
//! database and a mock Tiptap and Resend: only the relationship's coach, still a member of
//! its organization, may create a session in it.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{routing::post, Router};
use chrono::{NaiveDateTime, Utc};
use domain::{coaching_relationships, coaching_sessions, goals, organizations, Id};
use mockito::{Matcher, Mock, Server, ServerGuard};
use sea_orm::{MockDatabase, Value};
use serde_json::json;
use service::config::Config;

use super::create;
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, json_body, Caller, World,
};
use crate::AppState;

fn routes() -> Router<AppState> {
    Router::new().route("/coaching_sessions", post(create))
}

/// Tomorrow on the hour.
pub(crate) fn tomorrow() -> NaiveDateTime {
    (Utc::now() + chrono::Duration::days(1))
        .date_naive()
        .and_hms_opt(15, 0, 0)
        .expect("15:00 is a valid time")
}

fn create_body(coaching_relationship_id: Id) -> serde_json::Value {
    json!({
        "coaching_relationship_id": coaching_relationship_id,
        "date": tomorrow(),
        "duration_minutes": 60,
    })
}

/// Tiptap and Resend both served by `server`.
pub(crate) fn config(server: &ServerGuard) -> Config {
    Config::from_args([
        "test",
        &format!("--tiptap-url={}", server.url()),
        "--tiptap-auth-key=test_tiptap_key",
        "--resend-api-key=test_api_key_123",
        "--session-scheduled-email-template-id=session_template",
        "--recurring-sessions-scheduled-email-template-id=series_template",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={}", server.url()),
    ])
}

/// Refuses any outbound POST: no document created, no email sent.
pub(crate) async fn no_outbound_posts(server: &mut ServerGuard) -> Mock {
    server
        .mock("POST", Matcher::Any)
        .expect(0)
        .create_async()
        .await
}

pub(crate) fn organization(w: &World) -> organizations::Model {
    let now = Utc::now();
    organizations::Model {
        id: w.organization_id,
        name: "Acme".to_string(),
        logo: None,
        slug: "acme".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
        archived_at: None,
        archived_by: None,
    }
}

/// The participants who still belong to the relationship's organization.
fn members(w: &World) -> Vec<BTreeMap<String, Value>> {
    support::members(&[w.coach.id, w.coachee.id])
}

/// `caller` signed in, with R1 queued for the relationship check.
pub(crate) fn reaching_relationship(w: &World, caller: Caller) -> MockDatabase {
    w.signed_in(caller)
        .append_query_results([vec![w.relationship.clone()]])
}

/// Sends `POST /coaching_sessions` and asserts the refusal `expected` with nothing
/// written and nothing sent.
async fn assert_refused(
    row: &str,
    db: MockDatabase,
    email: &str,
    body: serde_json::Value,
    expected: StatusCode,
) {
    let mut server = Server::new_async().await;
    let outbound = no_outbound_posts(&mut server).await;
    let db = Arc::new(db.into_connection());
    let response = support::send_with(
        config(&server),
        routes(),
        &db,
        email,
        "POST",
        "/coaching_sessions",
        Some(body),
    )
    .await;
    assert_refusal(row, &db, response, expected);
    outbound.assert_async().await;
}

#[tokio::test]
async fn ses_09_coach_creates_a_session_in_the_relationship() {
    let w = support::world();
    let mut server = Server::new_async().await;
    let document = server
        .mock("POST", Matcher::Regex("^/api/documents/".to_string()))
        .with_status(200)
        .expect(1)
        .create_async()
        .await;
    let session = coaching_sessions::Model {
        id: Id::new_v4(),
        date: tomorrow(),
        ..w.session.clone()
    };
    let db = Arc::new(
        reaching_relationship(&w, Caller::Coach)
            .append_query_results([vec![w.relationship.clone()]])
            .append_query_results([vec![organization(&w)]])
            .append_query_results([members(&w)])
            .append_query_results([vec![session.clone()]])
            .append_query_results([Vec::<goals::Model>::new()])
            .append_query_results([Vec::<coaching_sessions::Model>::new()])
            .into_connection(),
    );
    let response = support::send_with(
        config(&server),
        routes(),
        &db,
        &w.coach.email,
        "POST",
        "/coaching_sessions",
        Some(create_body(w.relationship.id)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "SES-09");
    let body = json_body(response).await;
    assert_eq!(body["status_code"], json!(201), "SES-09");
    assert_eq!(body["data"]["id"], json!(session.id), "SES-09");
    document.assert_async().await;
}

#[tokio::test]
async fn ses_10_coachee_cannot_create_a_session() {
    let w = support::world();
    assert_refused(
        "SES-10",
        reaching_relationship(&w, Caller::Coachee),
        &w.coachee.email,
        create_body(w.relationship.id),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn ses_11_member_cannot_create_a_session() {
    let w = support::world();
    assert_refused(
        "SES-11",
        reaching_relationship(&w, Caller::Member),
        &w.member.email,
        create_body(w.relationship.id),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn ses_12_organization_admin_cannot_create_a_session() {
    let w = support::world();
    assert_refused(
        "SES-12",
        reaching_relationship(&w, Caller::Admin),
        &w.admin.email,
        create_body(w.relationship.id),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn ses_12b_super_admin_cannot_create_a_session() {
    let w = support::world();
    assert_refused(
        "SES-12b",
        reaching_relationship(&w, Caller::SuperAdmin),
        &w.super_admin.email,
        create_body(w.relationship.id),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn ses_13_outsider_cannot_create_a_session() {
    let w = support::world();
    assert_refused(
        "SES-13",
        reaching_relationship(&w, Caller::Outsider),
        &w.outsider.email,
        create_body(w.relationship.id),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn ses_14_removed_coachee_cannot_create_a_session() {
    let w = support::world();
    assert_refused(
        "SES-14",
        reaching_relationship(&w, Caller::RemovedCoachee),
        &w.coachee.email,
        create_body(w.relationship.id),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn ses_14b_removed_coach_cannot_create_a_session() {
    let w = support::world();
    assert_refused(
        "SES-14b",
        reaching_relationship(&w, Caller::Removed),
        &w.coach.email,
        create_body(w.relationship.id),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn ses_14c_unknown_relationship_is_not_found() {
    let w = support::world();
    assert_refused(
        "SES-14c",
        w.signed_in(Caller::Coach)
            .append_query_results([Vec::<coaching_relationships::Model>::new()]),
        &w.coach.email,
        create_body(Id::new_v4()),
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn m3_01_coach_cannot_schedule_with_a_coachee_no_longer_in_the_organization() {
    let w = support::world();
    let mut server = Server::new_async().await;
    let outbound = no_outbound_posts(&mut server).await;
    let db = Arc::new(
        reaching_relationship(&w, Caller::Coach)
            .append_query_results([vec![w.relationship.clone()]])
            .append_query_results([vec![organization(&w)]])
            .append_query_results([support::members(&[w.coach.id])])
            .into_connection(),
    );
    let response = support::send_with(
        config(&server),
        routes(),
        &db,
        &w.coach.email,
        "POST",
        "/coaching_sessions",
        Some(create_body(w.relationship.id)),
    )
    .await;
    let status = response.status();
    let body = json_body(response).await;
    assert_eq!(status, StatusCode::CONFLICT, "M3-01: {body}");
    support::assert_not_a_member(
        "M3-01",
        &body,
        "coachee_not_a_member",
        w.coachee.id,
        w.organization_id,
    );
    support::assert_nothing_written("M3-01", &db);
    outbound.assert_async().await;
}
