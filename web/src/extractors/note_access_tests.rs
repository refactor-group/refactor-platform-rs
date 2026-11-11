//! The note routes through the router, sign-in and access checks, against a mock database:
//! only a participant of the note's relationship may read, create or change it.

use std::ops::Deref;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{
    response::Response,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use domain::{coaching_relationships, coaching_sessions, notes, Id};
use sea_orm::{DatabaseConnection, MockDatabase};
use serde_json::json;

use crate::controller::note_controller::{create, read, update};
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, json_body, Caller,
};
use crate::AppState;

/// The shared relationship and session, with one note in the session.
struct World {
    base: support::World,
    note: notes::Model,
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
    let note = notes::Model {
        id: Id::new_v4(),
        coaching_session_id: base.session.id,
        body: Some("Talked through the plan".to_string()),
        user_id: base.coach.id,
        created_at: now.into(),
        updated_at: now.into(),
    };
    World { base, note }
}

impl World {
    /// `caller` signed in, with the note and its session queued for `NoteAccess`.
    fn reaching_note(&self, caller: Caller) -> MockDatabase {
        self.signed_in(caller)
            .append_query_results([vec![self.note.clone()]])
            .append_query_results([vec![(self.session.clone(), self.relationship.clone())]])
    }

    fn body(&self, coaching_session_id: Id) -> serde_json::Value {
        json!({
            "coaching_session_id": coaching_session_id,
            "body": "Revised the plan",
        })
    }
}

fn routes() -> Router<AppState> {
    Router::new()
        .route("/notes", post(create))
        .route("/notes/:id", get(read).put(update))
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
async fn note_04_coach_reads_the_note() {
    let w = world();
    let db = Arc::new(w.reaching_note(Caller::Coach).into_connection());
    let response = send(
        &db,
        &w.coach.email,
        "GET",
        &format!("/notes/{}", w.note.id),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "NOTE-04");
    let body = json_body(response).await;
    assert_eq!(body["data"]["id"], json!(w.note.id), "NOTE-04");
}

#[tokio::test]
async fn note_07_member_cannot_read_the_note() {
    let w = world();
    assert_refused(
        "NOTE-07",
        w.reaching_note(Caller::Member),
        &w.member.email,
        "GET",
        &format!("/notes/{}", w.note.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn note_07b_unknown_note_is_not_found() {
    let w = world();
    assert_refused(
        "NOTE-07b",
        w.signed_in(Caller::Coach)
            .append_query_results([Vec::<notes::Model>::new()]),
        &w.coach.email,
        "GET",
        &format!("/notes/{}", Id::new_v4()),
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn note_10_participant_removed_from_the_organization_cannot_read_the_note() {
    let w = world();
    assert_refused(
        "NOTE-10",
        w.reaching_note(Caller::Removed),
        &w.coach.email,
        "GET",
        &format!("/notes/{}", w.note.id),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn note_05_coachee_creates_a_note_in_the_session() {
    let w = world();
    let db = Arc::new(
        w.reaching_session(Caller::Coachee)
            // The INSERT ... RETURNING.
            .append_query_results([vec![w.note.clone()]])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coachee.email,
        "POST",
        "/notes",
        Some(w.body(w.session.id)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "NOTE-05");
    let body = json_body(response).await;
    assert_eq!(body["status_code"], json!(201), "NOTE-05");
}

#[tokio::test]
async fn note_08_outsider_cannot_create_a_note_in_the_session() {
    let w = world();
    assert_refused(
        "NOTE-08",
        w.reaching_session(Caller::Outsider),
        &w.outsider.email,
        "POST",
        "/notes",
        Some(w.body(w.session.id)),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn note_08b_unknown_session_is_not_found() {
    let w = world();
    assert_refused(
        "NOTE-08b",
        w.signed_in(Caller::Coach).append_query_results([Vec::<(
            coaching_sessions::Model,
            coaching_relationships::Model,
        )>::new()]),
        &w.coach.email,
        "POST",
        "/notes",
        Some(w.body(Id::new_v4())),
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn note_06_coach_updates_the_note() {
    let w = world();
    let db = Arc::new(
        w.reaching_note(Caller::Coach)
            // The read, then the UPDATE ... RETURNING.
            .append_query_results([vec![w.note.clone()]])
            .append_query_results([vec![w.note.clone()]])
            .into_connection(),
    );
    let response = send(
        &db,
        &w.coach.email,
        "PUT",
        &format!("/notes/{}", w.note.id),
        Some(w.body(w.session.id)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "NOTE-06");
}

#[tokio::test]
async fn note_09_member_cannot_update_the_note() {
    let w = world();
    assert_refused(
        "NOTE-09",
        w.reaching_note(Caller::Member),
        &w.member.email,
        "PUT",
        &format!("/notes/{}", w.note.id),
        Some(w.body(w.session.id)),
        StatusCode::FORBIDDEN,
    )
    .await;
}
