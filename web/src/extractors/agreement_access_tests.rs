//! The agreement routes through the router, sign-in and access checks, against a mock database:
//! only a participant of the agreement's relationship may read, create, change or delete it.

use std::ops::Deref;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{
    response::Response,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use domain::{agreements, coaching_relationships, coaching_sessions, Id};
use sea_orm::{DatabaseConnection, MockDatabase, MockExecResult};
use serde_json::json;

use crate::controller::agreement_controller::{create, delete, read, update};
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, json_body, Caller,
};
use crate::AppState;

/// The shared relationship and session, with one agreement in the session.
struct World {
    base: support::World,
    agreement: agreements::Model,
}

impl Deref for World {
    type Target = support::World;

    fn deref(&self) -> &support::World {
        &self.base
    }
}

fn world() -> World {
    let now = Utc::now();
    let base = support::world();
    let agreement = agreements::Model {
        id: Id::new_v4(),
        coaching_session_id: base.session.id,
        body: Some("Meet weekly".to_string()),
        user_id: base.coach.id,
        created_at: now.into(),
        updated_at: now.into(),
    };
    World { base, agreement }
}

impl World {
    /// `caller` signed in, with the agreement and its session queued for `AgreementAccess`.
    fn reaching_agreement(&self, caller: Caller) -> MockDatabase {
        self.signed_in(caller)
            .append_query_results([vec![self.agreement.clone()]])
            .append_query_results([vec![(self.session.clone(), self.relationship.clone())]])
    }

    fn body(&self, coaching_session_id: Id) -> serde_json::Value {
        json!({
            "coaching_session_id": coaching_session_id,
            "body": "Meet every other week",
        })
    }
}

fn routes() -> Router<AppState> {
    Router::new()
        .route("/agreements", post(create))
        .route("/agreements/:id", get(read).put(update).delete(delete))
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
async fn agr_04_coach_reads_the_agreement() {
    let w = world();
    let db = Arc::new(w.reaching_agreement(Caller::Coach).into_connection());
    let response = send(
        &db,
        &w.coach.email,
        "GET",
        &format!("/agreements/{}", w.agreement.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "AGR-04");
    let body = json_body(response).await;
    assert_eq!(body["data"]["id"], json!(w.agreement.id), "AGR-04");
}

#[tokio::test]
async fn agr_04b_coachee_reads_the_agreement() {
    let w = world();
    let db = Arc::new(w.reaching_agreement(Caller::Coachee).into_connection());
    let response = send(
        &db,
        &w.coachee.email,
        "GET",
        &format!("/agreements/{}", w.agreement.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "AGR-04b");
}

#[tokio::test]
async fn agr_08_member_cannot_read_the_agreement() {
    let w = world();
    assert_refused(
        "AGR-08",
        w.reaching_agreement(Caller::Member),
        &w.member.email,
        "GET",
        &format!("/agreements/{}", w.agreement.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn agr_08b_outsider_cannot_read_the_agreement() {
    let w = world();
    assert_refused(
        "AGR-08b",
        w.reaching_agreement(Caller::Outsider),
        &w.outsider.email,
        "GET",
        &format!("/agreements/{}", w.agreement.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn agr_08c_unknown_agreement_is_not_found() {
    let w = world();
    assert_refused(
        "AGR-08c",
        w.signed_in(Caller::Coach)
            .append_query_results([Vec::<agreements::Model>::new()]),
        &w.coach.email,
        "GET",
        &format!("/agreements/{}", Id::new_v4()),
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn agr_11_participant_removed_from_the_organization_cannot_read_the_agreement() {
    let w = world();
    assert_refused(
        "AGR-11",
        w.reaching_agreement(Caller::Removed),
        &w.coach.email,
        "GET",
        &format!("/agreements/{}", w.agreement.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn agr_05_coachee_creates_an_agreement_in_the_session() {
    let w = world();
    let db = Arc::new(
        w.reaching_session(Caller::Coachee)
            // The INSERT ... RETURNING.
            .append_query_results([vec![w.agreement.clone()]])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coachee.email,
        "POST",
        "/agreements",
        Some(w.body(w.session.id)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "AGR-05");
    let body = json_body(response).await;
    assert_eq!(body["status_code"], json!(201), "AGR-05");
}

#[tokio::test]
async fn agr_09_member_cannot_create_an_agreement_in_the_session() {
    let w = world();
    assert_refused(
        "AGR-09",
        w.reaching_session(Caller::Member),
        &w.member.email,
        "POST",
        "/agreements",
        Some(w.body(w.session.id)),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn agr_09b_unknown_session_is_not_found() {
    let w = world();
    assert_refused(
        "AGR-09b",
        w.signed_in(Caller::Coach).append_query_results([Vec::<(
            coaching_sessions::Model,
            coaching_relationships::Model,
        )>::new()]),
        &w.coach.email,
        "POST",
        "/agreements",
        Some(w.body(Id::new_v4())),
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn agr_06_coach_updates_the_agreement() {
    let w = world();
    let db = Arc::new(
        w.reaching_agreement(Caller::Coach)
            // The read, then the UPDATE ... RETURNING.
            .append_query_results([vec![w.agreement.clone()]])
            .append_query_results([vec![w.agreement.clone()]])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coach.email,
        "PUT",
        &format!("/agreements/{}", w.agreement.id),
        Some(w.body(w.session.id)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "AGR-06");
}

#[tokio::test]
async fn agr_10_member_cannot_update_the_agreement() {
    let w = world();
    assert_refused(
        "AGR-10",
        w.reaching_agreement(Caller::Member),
        &w.member.email,
        "PUT",
        &format!("/agreements/{}", w.agreement.id),
        Some(w.body(w.session.id)),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn agr_07_coachee_deletes_the_agreement() {
    let w = world();
    let db = Arc::new(
        w.reaching_agreement(Caller::Coachee)
            // `delete_by_id` reads the agreement, then `entity_api` reads it again to delete it.
            .append_query_results([vec![w.agreement.clone()]])
            .append_query_results([vec![w.agreement.clone()]])
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coachee.email,
        "DELETE",
        &format!("/agreements/{}", w.agreement.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "AGR-07");
}

#[tokio::test]
async fn agr_10b_outsider_cannot_delete_the_agreement() {
    let w = world();
    assert_refused(
        "AGR-10b",
        w.reaching_agreement(Caller::Outsider),
        &w.outsider.email,
        "DELETE",
        &format!("/agreements/{}", w.agreement.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}
