//! The app's real route table on in-memory SQLite, signed in as any fixture caller, with
//! local stand-ins for the external services it calls.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use axum::body::{to_bytes, Body, BodyDataStream, Bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use domain::gateway::object_storage::{self, ObjectStore};
use domain::test_utils::sqlite::access_world::{
    remove_roles, IMG1_BYTES, IMG1_CONTENT_TYPE, PASSWORD,
};
use domain::test_utils::sqlite::{
    database, mirror_migration_defaults, seed_access_world, AccessWorld, Caller,
};
use domain::user::Backend;
use domain::Id;
use futures::StreamExt;
use hmac::{Hmac, Mac};
use mockito::{Matcher, Mock, ServerGuard};
use sea_orm::DatabaseConnection;
use serde_json::{json, Value};
use service::config::Config;
use sha2::Sha256;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use crate::AppState;

const API_VERSION: &str = "1.0.0";

const JSON: (&str, &str) = ("content-type", "application/json");

/// The key Recall webhooks are signed with.
const WEBHOOK_KEY: &[u8] = b"access-test-webhook-key";

/// The matrix's "non-participants" for R1 and S1 data, all members of O1 except OUTSIDER.
pub(crate) const NON_PARTICIPANTS: [Caller; 5] = [
    Caller::Member,
    Caller::OtherCoach,
    Caller::Admin,
    Caller::Super,
    Caller::Outsider,
];

/// A set of fixture ids, for comparing with what a request returned.
pub(crate) fn ids<const N: usize>(fixture_ids: [Id; N]) -> BTreeSet<Id> {
    fixture_ids.into_iter().collect()
}

/// Tiptap's per-document endpoint, for both create and delete.
const TIPTAP_DOCUMENT: &str = r"^/api/documents/[^/?]+\?format=json$";

/// Resend's send endpoint.
const RESEND_SEND: &str = "/emails";

/// One seeded world behind the production router, with a session cookie per caller.
pub(crate) struct TestApp {
    app: Router,
    db: Arc<DatabaseConnection>,
    pub(crate) world: AccessWorld,
    cookies: HashMap<Caller, String>,
    stand_ins: StandIns,
}

/// A response: transport status, the raw body, and the body parsed (a JSON string when it is
/// not JSON).
pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) bytes: Bytes,
    pub(crate) body: Value,
}

/// Tiptap, Resend and AssemblyAI answered by one local server, and a local object store.
///
/// Every Tiptap and Resend call succeeds. AssemblyAI has no answer: the scan routes only need
/// its key set, and the server keeps a stray model call off the network.
struct StandIns {
    server: ServerGuard,
    /// Every address Resend was asked to mail, in send order.
    emailed: Emailed,
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
        let emailed = Emailed::default();
        resend_send(&mut server, &emailed)
            .expect_at_least(0)
            .create_async()
            .await;
        let store_root = StoreRoot::new();
        let config = config(&server.url(), &store_root);
        let store = object_storage::from_config(&config).expect("the local object store builds");
        Self {
            server,
            emailed,
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

/// The recipient addresses of every email sent.
type Emailed = Arc<Mutex<Vec<String>>>;

/// A Resend send, answered with the id Resend returns, recording its recipients.
fn resend_send(server: &mut ServerGuard, emailed: &Emailed) -> Mock {
    let emailed = emailed.clone();
    server
        .mock("POST", RESEND_SEND)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body_from_request(move |request| {
            let body: Value = request
                .body()
                .ok()
                .and_then(|bytes| serde_json::from_slice(bytes).ok())
                .unwrap_or_default();
            let to = body["to"].as_array().cloned().unwrap_or_default();
            emailed
                .lock()
                .expect("the recipient log locks")
                .extend(to.iter().filter_map(Value::as_str).map(address));
            br#"{"id":"stand-in"}"#.to_vec()
        })
}

/// The bare address of a recipient, written `"Name" <address>` or as the address alone.
fn address(recipient: &str) -> String {
    recipient
        .rsplit_once('<')
        .and_then(|(_, rest)| rest.strip_suffix('>'))
        .unwrap_or(recipient)
        .to_string()
}

/// A scratch directory for the local object store, removed when dropped.
struct StoreRoot(PathBuf);

impl StoreRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("access-test-{}", Id::new_v4()));
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
    pub(crate) fn ids(&self) -> BTreeSet<Id> {
        ids_in(&self.body["data"])
    }
}

/// Ids under each key of an object keyed by id, such as actions or goals per relationship.
pub(crate) fn ids_by_key(value: &Value) -> BTreeMap<Id, BTreeSet<Id>> {
    value
        .as_object()
        .expect("an object keyed by id")
        .iter()
        .map(|(key, items)| (key.parse().expect("an id key"), ids_in(items)))
        .collect()
}

/// The `id` of an object, or of every object in an array.
pub(crate) fn ids_in(value: &Value) -> BTreeSet<Id> {
    let id_of = |item: &Value| item["id"].as_str().and_then(|id| id.parse().ok());
    match value {
        Value::Array(items) => items.iter().filter_map(id_of).collect(),
        item => id_of(item).into_iter().collect(),
    }
}

impl TestApp {
    pub(crate) async fn new() -> Self {
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

    pub(crate) fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    /// Expects exactly `hits` Tiptap document calls with `method` from here on; check with
    /// `assert_async`.
    pub(crate) async fn expect_tiptap(&mut self, method: &str, hits: usize) -> Mock {
        tiptap_document(&mut self.stand_ins.server, method)
            .expect(hits)
            .create_async()
            .await
    }

    /// Expects exactly `hits` emails sent from here on; check with `assert_async`.
    pub(crate) async fn expect_emails(&mut self, hits: usize) -> Mock {
        resend_send(&mut self.stand_ins.server, &self.stand_ins.emailed)
            .expect(hits)
            .create_async()
            .await
    }

    /// The addresses mailed since the last call, sorted.
    pub(crate) fn take_emailed(&mut self) -> Vec<String> {
        let mut emailed = std::mem::take(
            &mut *self
                .stand_ins
                .emailed
                .lock()
                .expect("the recipient log locks"),
        );
        emailed.sort();
        emailed
    }

    /// One request: the status and the ids `data` returned.
    pub(crate) async fn expect(
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
    pub(crate) async fn send(
        &mut self,
        caller: Caller,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Reply {
        let body = body.map_or_else(Body::empty, |json| Body::from(json.to_string()));
        self.send_with(caller, method, path, &[JSON], body).await
    }

    /// Sends a request as `caller` with `headers` and a raw `body`.
    pub(crate) async fn send_with(
        &mut self,
        caller: Caller,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Body,
    ) -> Reply {
        let cookie = self.cookie(caller).await;
        let response = self
            .respond(Some(&cookie), method, path, headers, body)
            .await;
        read_reply(response).await
    }

    /// Opens `caller`'s event stream; the returned listener reads what reaches it.
    pub(crate) async fn listen(&mut self, caller: Caller) -> Listener {
        let cookie = self.cookie(caller).await;
        let response = self
            .respond(Some(&cookie), Method::GET, "/sse", &[], Body::empty())
            .await;
        assert_eq!(response.status(), StatusCode::OK, "{caller:?} listens");
        Listener(response.into_body().into_data_stream())
    }

    /// Delivers a Recall webhook, signed as Recall signs it; no one is signed in.
    pub(crate) async fn webhook(&mut self, event: &str, data: Value) -> StatusCode {
        let body = json!({ "event": event, "data": data }).to_string();
        let id = format!("msg_{}", Id::new_v4());
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let mut mac =
            Hmac::<Sha256>::new_from_slice(WEBHOOK_KEY).expect("an HMAC key of any length");
        mac.update(format!("{id}.{timestamp}.{body}").as_bytes());
        let signature = format!("v1,{}", BASE64.encode(mac.finalize().into_bytes()));
        let headers = [
            JSON,
            ("svix-id", id.as_str()),
            ("svix-timestamp", timestamp.as_str()),
            ("svix-signature", signature.as_str()),
        ];
        self.respond(
            None,
            Method::POST,
            "/webhooks/recall_ai",
            &headers,
            Body::from(body),
        )
        .await
        .status()
    }
}

/// A caller's open event stream.
pub(crate) struct Listener(BodyDataStream);

impl Listener {
    /// The events already sent to this stream, as `(event type, data)`.
    pub(crate) async fn received(&mut self) -> Vec<(String, Value)> {
        let mut events = Vec::new();
        while let Ok(Some(chunk)) =
            tokio::time::timeout(StdDuration::from_millis(100), self.0.next()).await
        {
            let chunk = chunk.expect("the stream reads");
            let text = String::from_utf8_lossy(&chunk);
            let field = |name: &str| {
                text.lines()
                    .find_map(|line| line.strip_prefix(name))
                    .map(str::trim)
            };
            if let (Some(event), Some(data)) = (field("event:"), field("data:")) {
                events.push((
                    event.to_string(),
                    serde_json::from_str(data).expect("event data is JSON"),
                ));
            }
        }
        events
    }

    /// The types of the events already sent to this stream.
    pub(crate) async fn received_types(&mut self) -> Vec<String> {
        self.received()
            .await
            .into_iter()
            .map(|(event, _)| event)
            .collect()
    }
}

/// A response read whole.
async fn read_reply(response: Response) -> Reply {
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

impl TestApp {
    /// GETs a streaming route as `caller`: the status and headers only. The body is never
    /// read, since a stream like `/sse` never ends.
    pub(crate) async fn open(&mut self, caller: Caller, path: &str) -> (StatusCode, HeaderMap) {
        let cookie = self.cookie(caller).await;
        let response = self
            .respond(Some(&cookie), Method::GET, path, &[], Body::empty())
            .await;
        (response.status(), response.headers().clone())
    }

    async fn respond(
        &mut self,
        cookie: Option<&str>,
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: Body,
    ) -> Response {
        let mut request = Request::builder()
            .uri(path)
            .method(method)
            .header("x-version", API_VERSION);
        if let Some(cookie) = cookie {
            request = request.header("cookie", cookie);
        }
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let mut request = request.body(body).expect("the request builds");
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
        "--tiptap-jwt-signing-key=access-test-signing-key",
        "--tiptap-app-id=access-test",
        &format!("--tiptap-url={server_url}"),
        "--tiptap-auth-key=access-test-tiptap-key",
        &format!("--resend-base-url={server_url}"),
        "--resend-api-key=access-test-resend-key",
        "--frontend-base-url=https://app.access.test",
        "--session-scheduled-email-template-id=session-scheduled",
        "--recurring-sessions-scheduled-email-template-id=recurring-sessions-scheduled",
        "--session-rescheduled-email-template-id=session-rescheduled",
        "--recurring-sessions-rescheduled-email-template-id=recurring-sessions-rescheduled",
        "--session-cancelled-email-template-id=session-cancelled",
        "--recurring-sessions-cancelled-email-template-id=recurring-sessions-cancelled",
        "--action-assigned-email-template-id=action-assigned",
        "--assembly-ai-api-key=access-test-assembly-ai-key",
        &format!("--assembly-ai-llm-base-url={server_url}"),
        &format!(
            "--recall-ai-webhook-secret=whsec_{}",
            BASE64.encode(WEBHOOK_KEY)
        ),
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
    let sse_manager = Arc::new(sse::Manager::default());
    let events = domain::events::EventPublisher::new().with_handler(Arc::new(
        sse::SseDomainEventHandler::new(sse_manager.clone()),
    ));
    let app_state = AppState::new(
        service::AppState::new(stand_ins.config.clone(), db),
        sse_manager,
        events,
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

/// `field` of every object in `value`, as ids.
pub(crate) fn field_ids(value: &Value, field: &str) -> BTreeSet<Id> {
    value
        .as_array()
        .expect("an array")
        .iter()
        .filter_map(|item| item[field].as_str().and_then(|id| id.parse().ok()))
        .collect()
}

/// COACH resends the invite at `path`; exactly COACH and COACHEE are mailed.
pub(crate) async fn assert_resend_mails_both(path: impl Fn(&TestApp) -> String) {
    let mut app = TestApp::new().await;
    let path = path(&app);
    app.take_emailed();

    let reply = app.send(Caller::Coach, Method::POST, &path, None).await;

    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let mut expected = vec![Caller::Coach.email(), Caller::Coachee.email()];
    expected.sort();
    assert_eq!(app.take_emailed(), expected);
}

/// COACH, holding no O1 role, resends the invite at `path`; refused, and nobody is mailed.
pub(crate) async fn assert_removed_coach_resend_refused(path: impl Fn(&TestApp) -> String) {
    let mut app = TestApp::new().await;
    let path = path(&app);
    let (coach, o1) = (app.world.user(Caller::Coach), app.world.o1);
    remove_roles(app.db(), coach, o1).await;
    app.take_emailed();

    let reply = app.send(Caller::Coach, Method::POST, &path, None).await;

    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(app.take_emailed(), Vec::<String>::new());
}

/// TRN-02, TRN-04 and TRN-06: every non-participant is refused each transcription read.
pub(crate) async fn assert_transcription_reads_refused(suffix: impl Fn(&TestApp) -> String) {
    let mut app = TestApp::new().await;
    let path = format!(
        "/coaching_sessions/{}/transcriptions{}",
        app.world.s1,
        suffix(&app)
    );

    for caller in NON_PARTICIPANTS {
        let (status, _) = app.expect(caller, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
    }
}
