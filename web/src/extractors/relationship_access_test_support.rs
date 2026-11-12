//! Shared scaffold for the relationship-scoped access tests: one relationship with one
//! session, callers inside and outside it, and a signed-in request through the router.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{body::Body, extract::Request, middleware::from_fn, response::Response, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::Utc;
use domain::user::Backend;
use domain::{coaching_relationships, coaching_sessions, user_roles, users, Id};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase, Value};
use service::config::Config;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use crate::middleware::auth::require_auth;
use crate::AppState;

fn user(email: &str) -> users::Model {
    let now = Utc::now();
    users::Model {
        id: Id::new_v4(),
        email: email.to_string(),
        first_name: "Test".to_string(),
        last_name: "User".to_string(),
        display_name: None,
        password: Some(generate_hash("password123")),
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: domain::duration::Duration::default_minutes(),
        working_hours_start: domain::working_hours::default_start(),
        working_hours_end: domain::working_hours::default_end(),
        roles: vec![],
        invite_status: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn role(user_id: Id, organization_id: Id) -> user_roles::Model {
    role_as(users::Role::User, user_id, Some(organization_id))
}

fn role_as(role: users::Role, user_id: Id, organization_id: Option<Id>) -> user_roles::Model {
    let now = Utc::now();
    user_roles::Model {
        id: Id::new_v4(),
        role,
        organization_id,
        user_id,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// A relationship with one session, plus callers outside it.
pub(crate) struct World {
    pub(crate) organization_id: Id,
    pub(crate) coach: users::Model,
    pub(crate) coachee: users::Model,
    pub(crate) super_admin: users::Model,
    pub(crate) relationship: coaching_relationships::Model,
    pub(crate) session: coaching_sessions::Model,
}

pub(crate) fn world() -> World {
    let now = Utc::now();
    let organization_id = Id::new_v4();
    let coach = user("coach@example.com");
    let coachee = user("coachee@example.com");
    let relationship = coaching_relationships::Model {
        id: Id::new_v4(),
        coach_id: coach.id,
        coachee_id: coachee.id,
        organization_id,
        slug: "test".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    };
    let session = coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id: relationship.id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: now.naive_utc(),
        duration_minutes: domain::duration::Duration::default_minutes(),
        title: None,
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: Some(now.into()),
        notice_given_at: now.into(),
    };
    World {
        organization_id,
        coach,
        coachee,
        super_admin: user("super@example.com"),
        relationship,
        session,
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Caller {
    Coach,
    Coachee,
    /// The coach, with no role in the relationship's organization.
    Removed,
    /// A SuperAdmin, not in the relationship.
    SuperAdmin,
}

impl World {
    fn caller(&self, caller: Caller) -> &users::Model {
        match caller {
            Caller::Coach | Caller::Removed => &self.coach,
            Caller::Coachee => &self.coachee,
            Caller::SuperAdmin => &self.super_admin,
        }
    }

    /// The login row and the per-request user row for `caller`.
    pub(crate) fn signed_in(&self, caller: Caller) -> MockDatabase {
        let user = self.caller(caller).clone();
        let db = MockDatabase::new(DatabaseBackend::Postgres);
        let role = match caller {
            Caller::Removed => {
                return db
                    .append_query_results([vec![(user.clone(), None::<user_roles::Model>)]])
                    .append_query_results([vec![(user, None::<user_roles::Model>)]])
            }
            Caller::SuperAdmin => role_as(users::Role::SuperAdmin, user.id, None),
            Caller::Coach | Caller::Coachee => role(user.id, self.organization_id),
        };
        db.append_query_results([vec![(user.clone(), role.clone())]])
            .append_query_results([vec![(user, role)]])
    }

    /// `caller` signed in, with the session queued for the create's session check.
    pub(crate) fn reaching_session(&self, caller: Caller) -> MockDatabase {
        self.signed_in(caller)
            .append_query_results([vec![(self.session.clone(), self.relationship.clone())]])
    }
}

/// Rows for the organization-membership filter, keeping exactly `user_ids`.
pub(crate) fn members(user_ids: &[Id]) -> Vec<BTreeMap<String, Value>> {
    user_ids
        .iter()
        .map(|&user_id| BTreeMap::from([("user_id".to_string(), user_id.into())]))
        .collect()
}

/// The app state over `db`, for routes layered with `from_fn_with_state`.
pub(crate) fn app_state(db: &Arc<DatabaseConnection>) -> AppState {
    app_state_with(Config::default(), db)
}

fn app_state_with(config: Config, db: &Arc<DatabaseConnection>) -> AppState {
    AppState::new(
        service::AppState::new(config, db),
        Arc::new(sse::Manager::default()),
        domain::events::EventPublisher::default(),
        None,
        None,
        None,
    )
}

/// The login route plus `routes` behind sign-in.
fn build_app(config: Config, db: Arc<DatabaseConnection>, routes: Router<AppState>) -> Router {
    let app_state = app_state_with(config, &db);

    let session_layer = SessionManagerLayer::new(MemoryStore::default())
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::days(1)))
        .with_always_save(true);
    let auth_layer = AuthManagerLayerBuilder::new(Backend::new(&db), session_layer).build();

    Router::new()
        .route(
            "/login",
            axum::routing::post(crate::controller::user_session_controller::login),
        )
        .merge(routes.route_layer(from_fn(require_auth)))
        .layer(auth_layer)
        .with_state(app_state)
}

/// Signs `email` in, then sends `method uri` with `body` as JSON to `routes`.
pub(crate) async fn send(
    routes: Router<AppState>,
    db: &Arc<DatabaseConnection>,
    email: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> Response {
    send_with(Config::default(), routes, db, email, method, uri, body).await
}

/// `send`, with the app running on `config`.
pub(crate) async fn send_with(
    config: Config,
    routes: Router<AppState>,
    db: &Arc<DatabaseConnection>,
    email: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> Response {
    let app = build_app(config, db.clone(), routes);
    let login = Request::builder()
        .uri("/login")
        .method("POST")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(format!("email={email}&password=password123")))
        .unwrap();
    let cookie = app
        .clone()
        .oneshot(login)
        .await
        .unwrap()
        .headers()
        .get("set-cookie")
        .and_then(|cookie| cookie.to_str().ok())
        .expect("login should return a session cookie")
        .to_string();

    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie)
        .header("x-version", "1.0.0")
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .unwrap();
    app.oneshot(request).await.unwrap()
}

pub(crate) async fn json_body(response: Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).expect("the body must be JSON")
}

/// Whether the database was asked to write anything.
fn wrote(db: &DatabaseConnection) -> bool {
    db.clone().into_transaction_log().iter().any(|t| {
        t.statements().iter().any(|s| {
            let sql = s.sql.trim_start().to_uppercase();
            ["INSERT", "UPDATE", "DELETE"]
                .iter()
                .any(|verb| sql.starts_with(verb))
        })
    })
}

/// Asserts the one UPDATE run set `status` to `value`, judged by its SET clause.
pub(crate) fn assert_status_written(row: &str, db: &DatabaseConnection, value: &str) {
    let set_clauses: Vec<String> = db
        .clone()
        .into_transaction_log()
        .iter()
        .flat_map(|t| t.statements().iter().map(|s| s.to_string()))
        .filter(|sql| sql.trim_start().to_uppercase().starts_with("UPDATE"))
        .filter_map(|sql| sql.split(" WHERE ").next().map(str::to_string))
        .collect();
    assert_eq!(set_clauses.len(), 1, "{row}: {set_clauses:?}");
    assert!(
        set_clauses[0].contains(&format!(r#""status" = CAST('{value}' AS "status")"#)),
        "{row}: {}",
        set_clauses[0]
    );
}

/// Asserts `response` is the refusal `expected` and nothing was written.
pub(crate) fn assert_refusal(
    row: &str,
    db: &DatabaseConnection,
    response: Response,
    expected: StatusCode,
) {
    assert_eq!(response.status(), expected, "{row}");
    assert_nothing_written(row, db);
}

/// Asserts the database was asked to write nothing.
pub(crate) fn assert_nothing_written(row: &str, db: &DatabaseConnection) {
    assert!(
        !wrote(db),
        "{row}: the refused request wrote to the database"
    );
}

/// Asserts `body` is the 409 refusing `user_id`, no longer in the organization, for `reason`.
pub(crate) fn assert_not_a_member(
    row: &str,
    body: &serde_json::Value,
    reason: &str,
    user_id: Id,
    organization_id: Id,
) {
    assert_eq!(body["error"], "conflict", "{row}: {body}");
    assert_eq!(body["details"]["reason"], reason, "{row}: {body}");
    assert_eq!(
        body["details"]["user_id"],
        serde_json::json!(user_id),
        "{row}: {body}"
    );
    assert_eq!(
        body["details"]["organization_id"],
        serde_json::json!(organization_id),
        "{row}: {body}"
    );
    assert!(
        body["message"].as_str().is_some_and(|m| !m.is_empty()),
        "{row}: {body}"
    );
}
