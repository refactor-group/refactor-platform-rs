//! A user's actions read through the router and `protect::users::actions`: a coach's query
//! is held to their own relationship with the user, a user's own query is not.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{middleware::from_fn_with_state, routing::get, Router};
use chrono::Utc;
use domain::{actions, Id};
use sea_orm::{DatabaseConnection, MockDatabase, Value};

use crate::controller::user::action_controller::index;
use crate::extractors::relationship_access_test_support::{self as support, Caller, World};
use crate::{protect, AppState};

/// Marks a WHERE clause that holds the relationship's coach to one user.
const COACH_FILTER: &str = r#"AND "coaching_relationships"."coach_id" ="#;

fn routes(db: &Arc<DatabaseConnection>) -> Router<AppState> {
    Router::new()
        .route("/users/:user_id/actions", get(index))
        .route_layer(from_fn_with_state(
            support::app_state(db),
            protect::users::actions::index,
        ))
}

/// One `actions_users` row for the coachee, so the assigned scope reaches its actions query.
fn assignment(w: &World) -> BTreeMap<&'static str, Value> {
    let now = Utc::now().fixed_offset();
    BTreeMap::from([
        ("id", Value::from(Id::new_v4())),
        ("action_id", Value::from(Id::new_v4())),
        ("user_id", Value::from(w.coachee.id)),
        ("created_at", Value::from(now)),
        ("updated_at", Value::from(now)),
    ])
}

/// `caller` signed in, past the middleware's coach check when they are the coach.
fn reaching_actions(w: &World, caller: Caller) -> MockDatabase {
    let db = w.signed_in(caller);
    match caller {
        Caller::Coach => db.append_query_results([vec![w.relationship.clone()]]),
        _ => db,
    }
}

/// GETs the coachee's actions as `email` and returns the actions query's WHERE clause.
async fn actions_where(w: &World, db: MockDatabase, email: &str, query: &str) -> String {
    let db = Arc::new(
        db.append_query_results([Vec::<actions::Model>::new()])
            .into_connection(),
    );
    let uri = format!("/users/{}/actions{query}", w.coachee.id);
    let response = support::send(routes(&db), &db, email, "GET", &uri, None).await;
    assert_eq!(response.status(), StatusCode::OK);

    let sql = (*db)
        .clone()
        .into_transaction_log()
        .iter()
        .flat_map(|t| t.statements().to_vec())
        .map(|s| s.sql)
        .find(|sql| sql.contains(r#"FROM "refactor_platform"."actions""#))
        .expect("the actions query ran");
    sql.split_once(" WHERE ")
        .map(|(_, clause)| clause.to_string())
        .expect("the actions query has a WHERE clause")
}

#[tokio::test]
async fn a_coach_reading_sessions_scope_is_held_to_their_relationship() {
    let w = support::world();
    let clause = actions_where(&w, reaching_actions(&w, Caller::Coach), &w.coach.email, "").await;
    assert!(clause.contains(COACH_FILTER), "{clause}");
    assert!(
        clause.contains(r#"AND "coaching_relationships"."coachee_id" ="#),
        "{clause}"
    );
}

#[tokio::test]
async fn a_coach_reading_assigned_scope_is_held_to_their_relationship() {
    let w = support::world();
    let db = reaching_actions(&w, Caller::Coach).append_query_results([vec![assignment(&w)]]);
    let clause = actions_where(&w, db, &w.coach.email, "?scope=assigned").await;
    assert!(
        clause.contains(r#""coaching_relationships"."coach_id" ="#),
        "{clause}"
    );
    assert!(
        clause.contains(r#""coaching_relationships"."coachee_id" ="#),
        "{clause}"
    );
}

#[tokio::test]
async fn a_user_reading_their_own_sessions_scope_is_not_held_to_a_coach() {
    let w = support::world();
    let clause = actions_where(
        &w,
        reaching_actions(&w, Caller::Coachee),
        &w.coachee.email,
        "",
    )
    .await;
    assert!(!clause.contains(COACH_FILTER), "{clause}");
}

#[tokio::test]
async fn a_user_reading_their_own_assigned_scope_is_not_held_to_a_coach() {
    let w = support::world();
    let db = reaching_actions(&w, Caller::Coachee).append_query_results([vec![assignment(&w)]]);
    let clause = actions_where(&w, db, &w.coachee.email, "?scope=assigned").await;
    assert!(!clause.contains("coaching_relationships"), "{clause}");
}
