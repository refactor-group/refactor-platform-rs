//! The batch session-goals read through the router, sign-in and `batch_by_session`, against a
//! mock database: every requested session must belong to a relationship of the caller.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::{middleware::from_fn_with_state, routing::get, Router};
use chrono::Utc;
use domain::{coaching_relationships, coaching_sessions, coaching_sessions_goals, Id};
use sea_orm::{DatabaseConnection, MockDatabase};

use super::batch_by_session;
use crate::controller::coaching_session::goal_controller::batch_index;
use crate::extractors::relationship_access_test_support::{
    self as support, assert_refusal, Caller, World,
};
use crate::AppState;

fn routes(db: &Arc<DatabaseConnection>) -> Router<AppState> {
    Router::new()
        .route("/coaching_sessions/goals", get(batch_index))
        .route_layer(from_fn_with_state(support::app_state(db), batch_by_session))
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
