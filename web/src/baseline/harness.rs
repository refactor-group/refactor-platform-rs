//! The app's real route table on in-memory SQLite, signed in as any fixture caller.

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use domain::test_utils::sqlite::access_world::PASSWORD;
use domain::test_utils::sqlite::{
    database, mirror_migration_defaults, seed_access_world, AccessWorld, Caller,
};
use domain::user::Backend;
use domain::Id;
use sea_orm::DatabaseConnection;
use serde_json::Value;
use service::config::Config;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use crate::AppState;

const API_VERSION: &str = "1.0.0";

/// The matrix's "non-participants" for R1 and S1 data, all members of O1 except OUTSIDER.
pub(super) const NON_PARTICIPANTS: [Caller; 5] = [
    Caller::Member,
    Caller::OtherCoach,
    Caller::Admin,
    Caller::Super,
    Caller::Outsider,
];

/// A set of fixture ids, for comparing with what a row returned.
pub(super) fn ids<const N: usize>(fixture_ids: [Id; N]) -> BTreeSet<Id> {
    fixture_ids.into_iter().collect()
}

/// One seeded world behind the production router, with a session cookie per caller.
pub(super) struct Baseline {
    app: Router,
    db: Arc<DatabaseConnection>,
    pub(super) world: AccessWorld,
    cookies: HashMap<Caller, String>,
}

/// A response: transport status and the parsed body (a JSON string when it is not JSON).
pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) body: Value,
}

impl Reply {
    /// Ids of the envelope's `data`: the object's own id, or each element's.
    pub(super) fn ids(&self) -> BTreeSet<Id> {
        ids_in(&self.body["data"])
    }
}

/// The `id` of an object, or of every object in an array.
pub(super) fn ids_in(value: &Value) -> BTreeSet<Id> {
    let id_of = |item: &Value| item["id"].as_str().and_then(|id| id.parse().ok());
    match value {
        Value::Array(items) => items.iter().filter_map(id_of).collect(),
        item => id_of(item).into_iter().collect(),
    }
}

impl Baseline {
    pub(super) async fn new() -> Self {
        let db = database().await;
        mirror_migration_defaults(&db).await;
        let db = Arc::new(db);
        let world = seed_access_world(&db).await;
        Self {
            app: app(&db),
            db,
            world,
            cookies: HashMap::new(),
        }
    }

    pub(super) fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    /// One matrix row: the status and the ids `data` returned.
    pub(super) async fn expect(
        &mut self,
        caller: Caller,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, BTreeSet<Id>) {
        let reply = self.send(caller, method, path, body).await;
        (reply.status, reply.ids())
    }

    /// Sends a request as `caller`, signing in first if needed.
    pub(super) async fn send(
        &mut self,
        caller: Caller,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Reply {
        let cookie = self.cookie(caller).await;
        let mut request = Request::builder()
            .uri(path)
            .method(method)
            .header("cookie", cookie)
            .header("x-version", API_VERSION)
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |json| Body::from(json.to_string())))
            .expect("the request builds");
        // Production serves with connect info; the throttled routes key on the peer address.
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4000))));
        let response = self
            .app
            .clone()
            .oneshot(request)
            .await
            .expect("the router answers");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body reads");
        Reply {
            status,
            body: serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        }
    }

    async fn cookie(&mut self, caller: Caller) -> String {
        if let Some(cookie) = self.cookies.get(&caller) {
            return cookie.clone();
        }
        let cookie = login(&self.app, caller).await;
        self.cookies.insert(caller, cookie.clone());
        cookie
    }
}

/// Defaults, plus a local Tiptap signing key so collaboration tokens can be minted.
fn config() -> Config {
    Config::from_args([
        "refactor-platform-rs",
        "--tiptap-jwt-signing-key",
        "access-baseline-signing-key",
        "--tiptap-app-id",
        "access-baseline",
    ])
}

/// The production route table behind the auth and session layers `init_server` adds.
/// The session store is in memory; production's lives in Postgres.
fn app(db: &Arc<DatabaseConnection>) -> Router {
    let app_state = AppState::new(
        service::AppState::new(config(), db),
        Arc::new(sse::Manager::default()),
        domain::events::EventPublisher::default(),
        None,
        None,
        None,
    );
    let session_layer = SessionManagerLayer::new(MemoryStore::default())
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::days(1)))
        .with_always_save(true);
    let auth_layer = AuthManagerLayerBuilder::new(Backend::new(db), session_layer).build();
    crate::router::define_routes(app_state).layer(auth_layer)
}

async fn login(app: &Router, caller: Caller) -> String {
    let form = format!(
        "email={}&password={}",
        urlencoding::encode(&caller.email()),
        urlencoding::encode(PASSWORD)
    );
    let request = Request::builder()
        .uri("/login")
        .method(Method::POST)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(form))
        .expect("the login request builds");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK, "{caller:?} signs in");
    response
        .headers()
        .get("set-cookie")
        .and_then(|cookie| cookie.to_str().ok())
        .expect("login sets a session cookie")
        .to_string()
}
