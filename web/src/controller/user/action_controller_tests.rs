//! A user's actions read through the router and `protect::users::actions`: a coach's query
//! is held to their own relationship with the user, a user's own query to their own
//! relationships, both in orgs the reader still belongs to unless a SuperAdmin.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{middleware::from_fn_with_state, routing::get, Router};
use chrono::Utc;
use domain::{actions, coaching_relationships, Id};
use sea_orm::{DatabaseConnection, MockDatabase, Value};

use crate::controller::user::action_controller::index;
use crate::extractors::relationship_access_test_support::{self as support, Caller, World};
use crate::{protect, AppState};

/// Marks a WHERE clause that holds the relationship's coach to one user.
const COACH_FILTER: &str = r#"AND "coaching_relationships"."coach_id" ="#;

/// Marks a WHERE clause that holds the reader to orgs they still belong to.
const ORG_FILTER: &str = r#""coaching_relationships"."organization_id" IN ("#;

/// What an empty membership list renders as instead: a constant false (`$a = $b`).
const NO_ORG: &str = " AND $";

fn routes(db: &Arc<DatabaseConnection>) -> Router<AppState> {
    Router::new()
        .route("/users/:user_id/actions", get(index))
        .route_layer(from_fn_with_state(
            support::app_state(db),
            protect::users::actions::index,
        ))
}

/// One `actions_users` row for `user_id`, so the assigned scope reaches its actions query.
fn assignment(user_id: Id) -> BTreeMap<&'static str, Value> {
    let now = Utc::now().fixed_offset();
    BTreeMap::from([
        ("id", Value::from(Id::new_v4())),
        ("action_id", Value::from(Id::new_v4())),
        ("user_id", Value::from(user_id)),
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

/// The SQL of every statement `db` ran.
fn statements(db: &DatabaseConnection) -> Vec<String> {
    db.clone()
        .into_transaction_log()
        .iter()
        .flat_map(|t| t.statements().to_vec())
        .map(|s| s.sql)
        .collect()
}

/// GETs the coachee's actions as `email` and returns the actions query's WHERE clause.
async fn actions_where(w: &World, db: MockDatabase, email: &str, query: &str) -> String {
    user_actions_where(w.coachee.id, db, email, query).await
}

/// GETs `user_id`'s actions as `email` and returns the actions query's WHERE clause.
async fn user_actions_where(user_id: Id, db: MockDatabase, email: &str, query: &str) -> String {
    let db = Arc::new(
        db.append_query_results([Vec::<actions::Model>::new()])
            .into_connection(),
    );
    let uri = format!("/users/{user_id}/actions{query}");
    let response = support::send(routes(&db), &db, email, "GET", &uri, None).await;
    assert_eq!(response.status(), StatusCode::OK);

    let sql = statements(&db)
        .into_iter()
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
    assert!(clause.contains(ORG_FILTER), "{clause}");
}

#[tokio::test]
async fn a_coach_reading_assigned_scope_is_held_to_their_relationship() {
    let w = support::world();
    let db =
        reaching_actions(&w, Caller::Coach).append_query_results([vec![assignment(w.coachee.id)]]);
    let clause = actions_where(&w, db, &w.coach.email, "?scope=assigned").await;
    assert!(
        clause.contains(r#""coaching_relationships"."coach_id" ="#),
        "{clause}"
    );
    assert!(
        clause.contains(r#""coaching_relationships"."coachee_id" ="#),
        "{clause}"
    );
    assert!(clause.contains(ORG_FILTER), "{clause}");
}

/// ACT-24i
#[tokio::test]
async fn a_coach_removed_from_the_relationships_org_is_forbidden() {
    let w = support::world();
    // What the guard's coach check returns once org membership is required.
    let db = Arc::new(
        w.signed_in(Caller::Removed)
            .append_query_results([Vec::<coaching_relationships::Model>::new()])
            .into_connection(),
    );
    let uri = format!("/users/{}/actions", w.coachee.id);
    let response = support::send(routes(&db), &db, &w.coach.email, "GET", &uri, None).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let sql = statements(&db);
    let check = sql
        .iter()
        .find_map(|sql| {
            sql.contains(r#"FROM "refactor_platform"."coaching_relationships""#)
                .then(|| sql.split_once(" WHERE "))
                .flatten()
        })
        .map(|(_, clause)| clause)
        .expect("the coach check ran");
    // An empty membership list still adds a condition that matches nothing.
    let after_pair = check
        .split_once(r#""coaching_relationships"."coachee_id" = "#)
        .map(|(_, rest)| rest)
        .expect("the check names the coachee");
    assert!(after_pair.contains(" AND "), "{check}");
    assert!(
        !sql.iter()
            .any(|sql| sql.contains(r#"FROM "refactor_platform"."actions""#)),
        "no actions query ran"
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
    assert!(clause.contains(ORG_FILTER), "{clause}");
}

#[tokio::test]
async fn a_user_reading_their_own_assigned_scope_is_not_held_to_a_coach() {
    let w = support::world();
    let db = reaching_actions(&w, Caller::Coachee)
        .append_query_results([vec![assignment(w.coachee.id)]]);
    let clause = actions_where(&w, db, &w.coachee.email, "?scope=assigned").await;
    assert!(!clause.contains(COACH_FILTER), "{clause}");
    assert!(clause.contains(ORG_FILTER), "{clause}");
}

#[tokio::test]
async fn a_super_admin_reading_their_own_actions_is_not_held_to_member_orgs() {
    let w = support::world();
    let id = w.super_admin.id;
    let sessions = user_actions_where(
        id,
        w.signed_in(Caller::SuperAdmin),
        &w.super_admin.email,
        "",
    )
    .await;
    assert!(
        !sessions.contains(ORG_FILTER) && !sessions.contains(NO_ORG),
        "{sessions}"
    );

    let db = w
        .signed_in(Caller::SuperAdmin)
        .append_query_results([vec![assignment(id)]]);
    let assigned = user_actions_where(id, db, &w.super_admin.email, "?scope=assigned").await;
    assert!(
        !assigned.contains(ORG_FILTER) && !assigned.contains(NO_ORG),
        "{assigned}"
    );
}
