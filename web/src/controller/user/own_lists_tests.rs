//! A user's own list routes through the router: each list query is held to the
//! organizations the caller still belongs to, unless the caller is a SuperAdmin.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::{middleware::from_fn_with_state, routing::get, Router};
use domain::coaching_relationships;
use sea_orm::{DatabaseConnection, MockDatabase};

use crate::controller::user::goal_controller;
use crate::controller::user::{coaching_relationships_controller, coaching_session_controller};
use crate::extractors::relationship_access_test_support::{self as support, Caller, World};
use crate::{protect, AppState};

/// Marks a WHERE clause that holds the caller to orgs they still belong to.
const ORG_FILTER: &str = r#""organization_id" IN ("#;

/// What an empty membership list renders as instead: a constant false (`$a = $b`).
const NO_ORG: &str = " AND $";

fn routes(db: &Arc<DatabaseConnection>) -> Router<AppState> {
    let state = support::app_state(db);
    Router::new()
        .merge(
            Router::new()
                .route(
                    "/users/:user_id/coaching-relationships",
                    get(coaching_relationships_controller::index),
                )
                .route_layer(from_fn_with_state(state.clone(), protect::users::read)),
        )
        .merge(
            Router::new()
                .route(
                    "/users/:user_id/coaching_sessions",
                    get(coaching_session_controller::index),
                )
                .route(
                    "/users/:user_id/coaching_sessions/counts",
                    get(coaching_session_controller::counts),
                )
                .route(
                    "/users/:user_id/coaching_sessions/nearest",
                    get(coaching_session_controller::nearest),
                )
                .route_layer(from_fn_with_state(
                    state.clone(),
                    protect::users::coaching_sessions::index,
                )),
        )
        .merge(
            Router::new()
                .route("/users/:user_id/goals", get(goal_controller::index))
                .route_layer(from_fn_with_state(state, protect::users::goals::index)),
        )
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

/// `caller` GETs their own `path?query`; returns the WHERE clause of the query on `table`.
async fn own_list_where(w: &World, caller: Caller, path: &str, query: &str, table: &str) -> String {
    let user = match caller {
        Caller::SuperAdmin => &w.super_admin,
        _ => &w.coachee,
    };
    // Empty results for the list query and anything that follows it.
    let db = Arc::new(
        (0..4)
            .fold(w.signed_in(caller), |db: MockDatabase, _| {
                db.append_query_results([Vec::<coaching_relationships::Model>::new()])
            })
            .into_connection(),
    );
    let uri = format!("/users/{}/{path}?{query}", user.id);
    let response = support::send(routes(&db), &db, &user.email, "GET", &uri, None).await;
    assert_eq!(response.status(), StatusCode::OK, "{uri}");

    let from = format!(r#"FROM "refactor_platform"."{table}""#);
    let sql = statements(&db)
        .into_iter()
        .find(|sql| sql.contains(&from))
        .unwrap_or_else(|| panic!("the {table} query ran"));
    sql.split_once(" WHERE ")
        .map(|(_, clause)| clause.to_string())
        .unwrap_or_else(|| panic!("the {table} query has a WHERE clause: {sql}"))
}

/// Asserts the org filter is on `route`'s query for a member and off for a SuperAdmin.
async fn assert_held_to_member_orgs(path: &str, query: &str, table: &str) {
    let w = support::world();
    let member = own_list_where(&w, Caller::Coachee, path, query, table).await;
    assert!(member.contains(ORG_FILTER), "member: {member}");
    let super_admin = own_list_where(&w, Caller::SuperAdmin, path, query, table).await;
    assert!(
        !super_admin.contains(ORG_FILTER) && !super_admin.contains(NO_ORG),
        "SuperAdmin: {super_admin}"
    );
}

#[tokio::test]
async fn own_relationships_are_held_to_member_orgs() {
    assert_held_to_member_orgs("coaching-relationships", "", "coaching_relationships").await;
}

#[tokio::test]
async fn own_sessions_are_held_to_member_orgs() {
    assert_held_to_member_orgs("coaching_sessions", "", "coaching_sessions").await;
}

#[tokio::test]
async fn own_nearest_sessions_are_held_to_member_orgs() {
    let relationship_id = support::world().relationship.id;
    let query = format!(
        "coaching_relationship_id={relationship_id}&around=2026-11-12T14:00:00Z&before=2&after=3"
    );
    assert_held_to_member_orgs("coaching_sessions/nearest", &query, "coaching_sessions").await;
}

#[tokio::test]
async fn own_session_counts_are_held_to_member_orgs() {
    assert_held_to_member_orgs(
        "coaching_sessions/counts",
        "from_date=2026-11-01&to_date=2026-11-30&group_by=month&tz=UTC",
        "coaching_sessions",
    )
    .await;
}

#[tokio::test]
async fn own_goals_are_held_to_member_orgs() {
    assert_held_to_member_orgs("goals", "", "goals").await;
}
