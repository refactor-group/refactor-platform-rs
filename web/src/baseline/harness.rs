//! The app's real route table on in-memory SQLite, signed in as any fixture caller, with
//! local stand-ins for the external services it calls.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::{to_bytes, Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use domain::gateway::object_storage::{self, ObjectStore};
use domain::test_utils::sqlite::access_world::{IMG1_BYTES, IMG1_CONTENT_TYPE, PASSWORD};
use domain::test_utils::sqlite::{
    database, mirror_migration_defaults, seed_access_world, AccessWorld, Caller,
};
use domain::user::Backend;
use domain::Id;
use mockito::{Matcher, Mock, ServerGuard};
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

/// Tiptap's per-document endpoint, for both create and delete.
const TIPTAP_DOCUMENT: &str = r"^/api/documents/[^/?]+\?format=json$";

/// Resend's send endpoint.
const RESEND_SEND: &str = "/emails";

/// One seeded world behind the production router, with a session cookie per caller.
pub(super) struct Baseline {
    app: Router,
    db: Arc<DatabaseConnection>,
    pub(super) world: AccessWorld,
    cookies: HashMap<Caller, String>,
    stand_ins: StandIns,
}

/// A response: transport status, the raw body, and the body parsed (a JSON string when it is
/// not JSON).
pub(super) struct Reply {
    pub(super) status: StatusCode,
    pub(super) bytes: Bytes,
    pub(super) body: Value,
}

/// Tiptap, Resend and AssemblyAI answered by one local server, and a local object store.
///
/// Every Tiptap and Resend call succeeds. AssemblyAI has no answer: the scan routes only need
/// its key set, and the server keeps a stray model call off the network.
struct StandIns {
    server: ServerGuard,
    config: Config,
    store: Arc<dyn ObjectStore>,
    store_root: StoreRoot,
}

impl StandIns {
    async fn start() -> Self {
        let mut server = mockito::Server::new_async().await;
        for method in ["POST", "DELETE"] {
            tiptap_document(&mut server, method)
                .expect_at_least(0)
                .create_async()
                .await;
        }
        resend_send(&mut server)
            .expect_at_least(0)
            .create_async()
            .await;
        let store_root = StoreRoot::new();
        let config = config(&server.url(), &store_root);
        let store = object_storage::from_config(&config).expect("the local object store builds");
        Self {
            server,
            config,
            store,
            store_root,
        }
    }
}

/// A Tiptap document call, answered with success.
fn tiptap_document(server: &mut ServerGuard, method: &str) -> Mock {
    server
        .mock(method, Matcher::Regex(TIPTAP_DOCUMENT.to_string()))
        .with_status(200)
}

/// A Resend send, answered with the id Resend returns.
fn resend_send(server: &mut ServerGuard) -> Mock {
    server
        .mock("POST", RESEND_SEND)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"id":"stand-in"}"#)
}

/// A scratch directory for the local object store, removed when dropped.
struct StoreRoot(PathBuf);

impl StoreRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("access-baseline-{}", Id::new_v4()));
        std::fs::create_dir_all(&path).expect("the object store root is created");
        Self(path)
    }
}

impl Drop for StoreRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Reply {
    /// Ids of the envelope's `data`: the object's own id, or each element's.
    pub(super) fn ids(&self) -> BTreeSet<Id> {
        ids_in(&self.body["data"])
    }
}

/// Ids under each key of an object keyed by id, such as actions or goals per relationship.
pub(super) fn ids_by_key(value: &Value) -> BTreeMap<Id, BTreeSet<Id>> {
    value
        .as_object()
        .expect("an object keyed by id")
        .iter()
        .map(|(key, items)| (key.parse().expect("an id key"), ids_in(items)))
        .collect()
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
        let stand_ins = StandIns::start().await;
        store_img1(&stand_ins, &world).await;
        Self {
            app: app(&db, &stand_ins),
            db,
            world,
            cookies: HashMap::new(),
            stand_ins,
        }
    }

    pub(super) fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    /// Expects exactly `hits` Tiptap document calls with `method` from here on; check with
    /// `assert_async`.
    pub(super) async fn expect_tiptap(&mut self, method: &str, hits: usize) -> Mock {
        tiptap_document(&mut self.stand_ins.server, method)
            .expect(hits)
            .create_async()
            .await
    }

    /// Expects exactly `hits` emails sent from here on; check with `assert_async`.
    pub(super) async fn expect_emails(&mut self, hits: usize) -> Mock {
        resend_send(&mut self.stand_ins.server)
            .expect(hits)
            .create_async()
            .await
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
        let response = self.respond(caller, method, path, body).await;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the body reads");
        Reply {
            status,
            body: serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
            bytes,
        }
    }

    /// GETs a streaming route as `caller`: the status and headers only. The body is never
    /// read, since a stream like `/sse` never ends.
    pub(super) async fn open(&mut self, caller: Caller, path: &str) -> (StatusCode, HeaderMap) {
        let response = self.respond(caller, Method::GET, path, None).await;
        (response.status(), response.headers().clone())
    }

    async fn respond(
        &mut self,
        caller: Caller,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Response {
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
        self.app
            .clone()
            .oneshot(request)
            .await
            .expect("the router answers")
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

/// Defaults, a local Tiptap signing key so collaboration tokens can be minted, and every
/// external service pointed at the stand-ins: Tiptap, Resend and AssemblyAI at `server_url`,
/// object storage on local disk under `store_root`.
fn config(server_url: &str, store_root: &StoreRoot) -> Config {
    let store_root = store_root.0.to_string_lossy();
    Config::from_args([
        "refactor-platform-rs",
        "--tiptap-jwt-signing-key=access-baseline-signing-key",
        "--tiptap-app-id=access-baseline",
        &format!("--tiptap-url={server_url}"),
        "--tiptap-auth-key=access-baseline-tiptap-key",
        &format!("--resend-base-url={server_url}"),
        "--resend-api-key=access-baseline-resend-key",
        "--frontend-base-url=https://app.access.test",
        "--session-scheduled-email-template-id=session-scheduled",
        "--recurring-sessions-scheduled-email-template-id=recurring-sessions-scheduled",
        "--session-rescheduled-email-template-id=session-rescheduled",
        "--recurring-sessions-rescheduled-email-template-id=recurring-sessions-rescheduled",
        "--session-cancelled-email-template-id=session-cancelled",
        "--recurring-sessions-cancelled-email-template-id=recurring-sessions-cancelled",
        "--action-assigned-email-template-id=action-assigned",
        "--assembly-ai-api-key=access-baseline-assembly-ai-key",
        &format!("--assembly-ai-llm-base-url={server_url}"),
        "--object-store-backend=local",
        &format!("--object-store-local-path={store_root}"),
    ])
}

/// Puts IMG1's bytes in the store at its storage key.
async fn store_img1(stand_ins: &StandIns, world: &AccessWorld) {
    let source = stand_ins.store_root.0.join("img1-upload");
    std::fs::write(&source, IMG1_BYTES).expect("IMG1's upload is written");
    stand_ins
        .store
        .put(&world.img1_storage_key, &source, IMG1_CONTENT_TYPE)
        .await
        .expect("IMG1 is stored");
}

/// The production route table behind the auth and session layers `init_server` adds.
/// The session store is in memory; production's lives in Postgres.
fn app(db: &Arc<DatabaseConnection>, stand_ins: &StandIns) -> Router {
    let app_state = AppState::new(
        service::AppState::new(stand_ins.config.clone(), db),
        Arc::new(sse::Manager::default()),
        domain::events::EventPublisher::default(),
        None,
        None,
        Some(stand_ins.store.clone()),
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
