//! A user's own session counts query is held to the organizations the caller still belongs
//! to, unless the caller is a SuperAdmin. Each own-list handler also refuses another user's
//! path id on its own, without the guard.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{middleware::from_fn_with_state, routing::get, Router};
use domain::coaching_relationships;
use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase};
use serde::de::DeserializeOwned;

use crate::controller::user::goal_controller;
use crate::controller::user::{coaching_relationships_controller, coaching_session_controller};
use crate::extractors::authenticated_user::AuthenticatedUser;
use crate::extractors::compare_api_version::CompareApiVersion;
use crate::extractors::relationship_access_test_support::{self as support, Caller, Fixture};
use crate::{protect, AppState};

/// Marks a WHERE clause that holds the caller to orgs they still belong to.
const ORG_FILTER: &str = r#""organization_id" IN ("#;

/// What an empty membership list renders as instead: a constant false (`$a = $b`).
const NO_ORG: &str = " AND $";

fn routes(db: &Arc<DatabaseConnection>) -> Router<AppState> {
    Router::new()
        .route(
            "/users/:user_id/coaching_sessions/counts",
            get(coaching_session_controller::counts),
        )
        .route_layer(from_fn_with_state(
            support::app_state(db),
            protect::users::coaching_sessions::index,
        ))
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
async fn own_list_where(
    w: &Fixture,
    caller: Caller,
    path: &str,
    query: &str,
    table: &str,
) -> String {
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
    let w = support::fixture();
    let member = own_list_where(&w, Caller::Coachee, path, query, table).await;
    assert!(member.contains(ORG_FILTER), "member: {member}");
    let super_admin = own_list_where(&w, Caller::SuperAdmin, path, query, table).await;
    assert!(
        !super_admin.contains(ORG_FILTER) && !super_admin.contains(NO_ORG),
        "SuperAdmin: {super_admin}"
    );
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

/// A handler's own parts for the coachee asking about the coach's id, against a database
/// that answers nothing.
struct Foreign {
    w: Fixture,
    db: Arc<DatabaseConnection>,
}

impl Foreign {
    fn new() -> Self {
        Self {
            w: support::fixture(),
            db: Arc::new(MockDatabase::new(DatabaseBackend::Postgres).into_connection()),
        }
    }

    fn version(&self) -> CompareApiVersion {
        CompareApiVersion(HeaderValue::from_static("1.0.0"))
    }

    fn caller(&self) -> AuthenticatedUser {
        AuthenticatedUser(self.w.coachee.clone())
    }

    fn state(&self) -> State<AppState> {
        State(support::app_state(&self.db))
    }

    fn path(&self) -> Path<domain::Id> {
        Path(self.w.coach.id)
    }

    /// Asserts `response` is the guard's 403 and nothing reached the database.
    async fn assert_refused(&self, response: Response) {
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body reads");
        assert_eq!(&body[..], b"FORBIDDEN");
        assert!(
            statements(&self.db).is_empty(),
            "{:?}",
            statements(&self.db)
        );
    }
}

fn query<T: DeserializeOwned>(query: &str) -> Query<T> {
    let uri: Uri = format!("/?{query}").parse().expect("a valid uri");
    Query::try_from_uri(&uri).expect("the query parses")
}

/// M8-03: the sessions list refuses another user's path id before any query.
#[tokio::test]
async fn m8_03_sessions_list_refuses_another_users_path_id() {
    let f = Foreign::new();
    let response =
        coaching_session_controller::index(f.version(), f.caller(), f.state(), f.path(), query(""))
            .await
            .into_response();
    f.assert_refused(response).await;
}

/// M8-03: the nearest sessions refuse another user's path id before any query.
#[tokio::test]
async fn m8_03_nearest_sessions_refuse_another_users_path_id() {
    let f = Foreign::new();
    let window = format!(
        "coaching_relationship_id={}&around=2026-11-12T14:00:00Z&before=2&after=3",
        f.w.relationship.id
    );
    let response = coaching_session_controller::nearest(
        f.version(),
        f.caller(),
        f.state(),
        f.path(),
        query(&window),
    )
    .await
    .into_response();
    f.assert_refused(response).await;
}

/// M8-03: the session counts refuse another user's path id before any query.
#[tokio::test]
async fn m8_03_session_counts_refuse_another_users_path_id() {
    let f = Foreign::new();
    let response = coaching_session_controller::counts(
        f.version(),
        f.caller(),
        f.state(),
        f.path(),
        query("from_date=2026-11-01&to_date=2026-11-30&group_by=month&tz=UTC"),
    )
    .await
    .into_response();
    f.assert_refused(response).await;
}

/// M8-03: the relationships list refuses another user's path id before any query.
#[tokio::test]
async fn m8_03_relationships_list_refuses_another_users_path_id() {
    let f = Foreign::new();
    let response = coaching_relationships_controller::index(
        f.version(),
        f.caller(),
        f.state(),
        f.path(),
        query(""),
    )
    .await
    .into_response();
    f.assert_refused(response).await;
}

/// M8-03: the goals list refuses another user's path id before any query.
#[tokio::test]
async fn m8_03_goals_list_refuses_another_users_path_id() {
    let f = Foreign::new();
    let response = goal_controller::index(f.version(), f.caller(), f.state(), f.path(), query(""))
        .await
        .into_response();
    f.assert_refused(response).await;
}
