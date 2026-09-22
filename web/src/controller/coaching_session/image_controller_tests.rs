use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use axum::http::StatusCode;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Request},
    middleware::from_fn,
    routing::{get, post},
    Router,
};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use chrono::{NaiveDate, Utc};
use domain::error::Error as DomainError;
use domain::gateway::object_storage::{ObjectStore, StoredObject};
use domain::user::Backend;
use domain::{
    coaching_relationships, coaching_session_images, coaching_sessions, user_roles, users, Id,
};
use password_auth::generate_hash;
use sea_orm::{DatabaseBackend, MockDatabase};
use service::config::Config;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use super::{create, delete, read, restore};
use crate::middleware::auth::require_auth;
use crate::AppState;

const UPLOAD_ROUTE: &str = "/coaching_sessions/:coaching_session_id/images";
const FETCH_ROUTE: &str = "/coaching_session_images/:image_id";
const RESTORE_ROUTE: &str = "/coaching_session_images/:image_id/restore";
const BOUNDARY: &str = "note-image-test-boundary";
const PRESIGNED_URL: &str = "https://example.digitaloceanspaces.com/key?X-Amz-Signature=abc";

/// A valid 2x3 truecolor PNG, so `inspect_image` sees a real image rather than a fixture file.
const TINY_PNG: [u8; 73] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x08, 0x02, 0x00, 0x00, 0x00, 0x36, 0x88, 0x49,
    0xd6, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x44, 0x0c, 0x28, 0x14, 0x00, 0x44, 0xd0, 0x05, 0xfb, 0xa4, 0xcf, 0xde, 0x80, 0x00, 0x00, 0x00,
    0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// Stands in for a storage backend without touching the network. `presigns` picks which
/// half of `read` runs: Spaces-shaped (a signed URL) or local-shaped (bytes to stream).
struct TestObjectStore {
    presigns: bool,
}

#[async_trait]
impl ObjectStore for TestObjectStore {
    async fn put(
        &self,
        _key: &str,
        _bytes: Vec<u8>,
        _content_type: &str,
    ) -> Result<(), DomainError> {
        Ok(())
    }

    fn presigned_get(&self, _key: &str, _ttl: StdDuration) -> Result<Option<String>, DomainError> {
        Ok(self.presigns.then(|| PRESIGNED_URL.to_string()))
    }

    async fn get(&self, _key: &str) -> Result<StoredObject, DomainError> {
        Ok(StoredObject {
            bytes: TINY_PNG.to_vec(),
            content_type: "image/png".to_string(),
        })
    }

    async fn delete(&self, _key: &str) -> Result<(), DomainError> {
        Ok(())
    }
}

fn coach() -> users::Model {
    let now = Utc::now();
    users::Model {
        id: Id::new_v4(),
        email: "test@example.com".to_string(),
        first_name: "Test".to_string(),
        last_name: "User".to_string(),
        display_name: Some("Test User".to_string()),
        password: Some(generate_hash("password123")),
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: domain::duration::Duration::default_minutes(),
        roles: vec![],
        invite_status: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn role(user_id: Id, organization_id: Id) -> user_roles::Model {
    let now = Utc::now();
    user_roles::Model {
        id: Id::new_v4(),
        role: users::Role::User,
        organization_id: Some(organization_id),
        user_id,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn relationship(
    organization_id: Id,
    coach_id: Id,
    coachee_id: Id,
) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        id: Id::new_v4(),
        coach_id,
        coachee_id,
        organization_id,
        slug: "test".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn session(relationship_id: Id) -> coaching_sessions::Model {
    let now = Utc::now();
    coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id: relationship_id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: NaiveDate::from_ymd_opt(2026, 9, 21)
            .and_then(|date| date.and_hms_opt(10, 0, 0))
            .expect("the fixture date must be valid"),
        duration_minutes: domain::duration::Duration::default_minutes(),
        title: None,
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: Some(now.into()),
        notice_given_at: now.into(),
    }
}

fn image(session_id: Id, uploaded_by_id: Id, deleted: bool) -> coaching_session_images::Model {
    let now = Utc::now();
    coaching_session_images::Model {
        id: Id::new_v4(),
        coaching_session_id: session_id,
        uploaded_by_id,
        storage_key: "coaching-sessions/abc/images/def.png".to_string(),
        mime_type: "image/png".to_string(),
        byte_size: TINY_PNG.len() as i64,
        width: Some(2),
        height: Some(3),
        deleted_at: deleted.then(|| now.into()),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn build_app(
    db: Arc<sea_orm::DatabaseConnection>,
    object_store: Option<Arc<dyn ObjectStore>>,
) -> Router {
    let app_state = AppState::new(
        service::AppState::new(Config::default(), &db),
        Arc::new(sse::Manager::default()),
        domain::events::EventPublisher::default(),
        None,
        None,
        object_store,
    );

    let session_layer = SessionManagerLayer::new(MemoryStore::default())
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::days(1)))
        .with_always_save(true);
    let auth_layer = AuthManagerLayerBuilder::new(Backend::new(&db), session_layer).build();

    let body_limit = usize::try_from(
        app_state
            .config
            .coaching_session_image_max_bytes()
            .saturating_add(64 * 1024),
    )
    .unwrap_or(usize::MAX);

    Router::new()
        .route(
            "/login",
            axum::routing::post(crate::controller::user_session_controller::login),
        )
        .merge(
            Router::new()
                .route(
                    UPLOAD_ROUTE,
                    post(create).layer(DefaultBodyLimit::max(body_limit)),
                )
                .route(FETCH_ROUTE, get(read).delete(delete))
                .route(RESTORE_ROUTE, post(restore))
                .route_layer(from_fn(require_auth)),
        )
        .layer(auth_layer)
        .with_state(app_state)
}

fn presigning_store() -> Option<Arc<dyn ObjectStore>> {
    Some(Arc::new(TestObjectStore { presigns: true }))
}

fn streaming_store() -> Option<Arc<dyn ObjectStore>> {
    Some(Arc::new(TestObjectStore { presigns: false }))
}

async fn login_cookie(app: &Router) -> String {
    let login_request = Request::builder()
        .uri("/login")
        .method("POST")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("email=test@example.com&password=password123"))
        .unwrap();
    let login_response = app.clone().oneshot(login_request).await.unwrap();
    login_response
        .headers()
        .get("set-cookie")
        .and_then(|cookie| cookie.to_str().ok())
        .expect("login should return a session cookie")
        .to_string()
}

/// The auth rows every protected request consumes: one for login, one for the session restore.
fn authenticated(user: &users::Model, role: &user_roles::Model) -> MockDatabase {
    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![(user.clone(), role.clone())]])
        .append_query_results([vec![(user.clone(), role.clone())]])
}

/// One `multipart/form-data` body carrying a single part.
fn multipart_body(part_name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; \
         name=\"{part_name}\"; filename=\"paste.png\"\r\n\
         Content-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

async fn upload(app: &Router, cookie: &str, session_id: Id, body: Vec<u8>) -> StatusCode {
    upload_response(app, cookie, session_id, body)
        .await
        .status()
}

async fn upload_response(
    app: &Router,
    cookie: &str,
    session_id: Id,
    body: Vec<u8>,
) -> axum::response::Response {
    let request = Request::builder()
        .uri(format!("/coaching_sessions/{session_id}/images"))
        .method("POST")
        .header("cookie", cookie)
        .header("x-version", "1.0.0-beta1")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .unwrap();

    app.clone().oneshot(request).await.unwrap()
}

/// Fetches an image. Deliberately sends no `x-version`: an `<img>` tag cannot.
async fn fetch(app: &Router, cookie: Option<&str>, image_id: Id) -> axum::response::Response {
    let request = Request::builder().uri(format!("/coaching_session_images/{image_id}"));
    let request = match cookie {
        Some(cookie) => request.header("cookie", cookie),
        None => request,
    };

    app.clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

fn header(response: &axum::response::Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

/// Everything a single request needs, wired for one participation outcome.
struct World {
    app: Router,
    cookie: String,
    session_id: Id,
    image_id: Id,
}

/// Builds a fetch world. `participant` decides whether the logged-in user is the coach of
/// the image's session or a stranger holding a valid image id.
async fn fetch_world(participant: bool, object_store: Option<Arc<dyn ObjectStore>>) -> World {
    fetch_world_for(participant, object_store, false).await
}

/// As `fetch_world`, with control over whether the stored row is already marked deleted.
async fn fetch_world_for(
    participant: bool,
    object_store: Option<Arc<dyn ObjectStore>>,
    deleted: bool,
) -> World {
    let organization_id = Id::new_v4();
    let user = coach();
    let role = role(user.id, organization_id);
    let coach_id = if participant { user.id } else { Id::new_v4() };
    let relationship = relationship(organization_id, coach_id, Id::new_v4());
    let session = session(relationship.id);
    let image = image(session.id, coach_id, deleted);

    let db = authenticated(&user, &role)
        .append_query_results([vec![image.clone()]])
        .append_query_results([vec![(session.clone(), relationship.clone())]])
        .into_connection();

    let app = build_app(Arc::new(db), object_store);
    let cookie = login_cookie(&app).await;

    World {
        app,
        cookie,
        session_id: session.id,
        image_id: image.id,
    }
}

/// Builds an upload world, with the same participation switch.
async fn upload_world(participant: bool, object_store: Option<Arc<dyn ObjectStore>>) -> World {
    let organization_id = Id::new_v4();
    let user = coach();
    let role = role(user.id, organization_id);
    let coach_id = if participant { user.id } else { Id::new_v4() };
    let relationship = relationship(organization_id, coach_id, Id::new_v4());
    let session = session(relationship.id);
    let image = image(session.id, coach_id, false);

    let db = authenticated(&user, &role)
        .append_query_results([vec![(session.clone(), relationship.clone())]])
        .append_query_results([vec![image.clone()]])
        .into_connection();

    let app = build_app(Arc::new(db), object_store);
    let cookie = login_cookie(&app).await;

    World {
        app,
        cookie,
        session_id: session.id,
        image_id: image.id,
    }
}

/// The `max-age` a `Cache-Control` header grants, in seconds.
fn max_age(response: &axum::response::Response) -> u64 {
    header(response, "cache-control")
        .split(',')
        .filter_map(|directive| {
            directive
                .trim()
                .strip_prefix("max-age=")
                .map(str::to_string)
        })
        .find_map(|seconds| seconds.parse::<u64>().ok())
        .expect("the response must grant a parseable max-age")
}

#[tokio::test]
async fn a_participant_is_redirected_to_a_presigned_url() {
    let world = fetch_world(true, presigning_store()).await;

    let response = fetch(&world.app, Some(&world.cookie), world.image_id).await;

    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(header(&response, "location"), PRESIGNED_URL);
    assert_eq!(header(&response, "vary"), "Cookie");
}

/// The read handler must never gain `CompareApiVersion`: a browser `<img>` tag cannot send
/// `x-version`, so requiring it would break every image in the product. This request sends
/// none and must still be served.
#[tokio::test]
async fn a_fetch_without_an_api_version_header_is_served() {
    let world = fetch_world(true, presigning_store()).await;

    let status = fetch(&world.app, Some(&world.cookie), world.image_id)
        .await
        .status();

    assert!(
        status == StatusCode::FOUND || status == StatusCode::OK,
        "a versionless fetch must be served, got {status}"
    );
}

/// The invariant, not today's numbers: a cached redirect must expire before the signature
/// it carries, whatever the configured TTL happens to be.
#[tokio::test]
async fn the_cache_lifetime_is_shorter_than_the_presign_ttl() {
    let world = fetch_world(true, presigning_store()).await;
    let presign_ttl = Config::default().coaching_session_image_presign_ttl_seconds();

    let response = fetch(&world.app, Some(&world.cookie), world.image_id).await;

    assert!(
        max_age(&response) < presign_ttl,
        "max-age {} must be strictly less than the presign TTL {presign_ttl}",
        max_age(&response)
    );
}

/// A backend that cannot sign (the local filesystem) streams the bytes instead, and the
/// cache directives still apply.
#[tokio::test]
async fn a_backend_that_cannot_presign_streams_the_bytes() {
    let world = fetch_world(true, streaming_store()).await;
    let presign_ttl = Config::default().coaching_session_image_presign_ttl_seconds();

    let response = fetch(&world.app, Some(&world.cookie), world.image_id).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), "image/png");
    assert_eq!(header(&response, "vary"), "Cookie");
    assert!(max_age(&response) < presign_ttl);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(bytes.as_ref(), TINY_PNG);
}

/// The id lives forever inside opaque CRDT note bytes, so it must not act as a bearer
/// token: holding a valid one buys a non-participant nothing.
#[tokio::test]
async fn a_non_participant_cannot_fetch_an_image_it_holds_the_id_for() {
    let world = fetch_world(false, presigning_store()).await;

    let status = fetch(&world.app, Some(&world.cookie), world.image_id)
        .await
        .status();

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "a non-participant must be refused, got {status}"
    );
}

#[tokio::test]
async fn an_unauthenticated_fetch_is_unauthorized() {
    let world = fetch_world(true, presigning_store()).await;

    let status = fetch(&world.app, None, world.image_id).await.status();

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_non_participant_cannot_upload() {
    let world = upload_world(false, presigning_store()).await;

    let status = upload(
        &world.app,
        &world.cookie,
        world.session_id,
        multipart_body("file", &TINY_PNG),
    )
    .await;

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "a non-participant must be refused, got {status}"
    );
}

/// The envelope carries 201, matching every other create in this API, and the storage key
/// stays behind: it is `#[serde(skip)]` so the bucket layout never reaches a client.
#[tokio::test]
async fn a_participant_uploads_an_image() {
    let world = upload_world(true, presigning_store()).await;

    let response = upload_response(
        &world.app,
        &world.cookie,
        world.session_id,
        multipart_body("file", &TINY_PNG),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&bytes).expect("the response must be JSON");

    assert_eq!(body["status_code"], 201);
    assert_eq!(body["data"]["mime_type"], "image/png");
    assert!(
        body["data"].get("storage_key").is_none(),
        "the storage key must not cross the wire: {body}"
    );
}

#[tokio::test]
async fn an_upload_without_a_file_part_is_a_bad_request() {
    let world = upload_world(true, presigning_store()).await;

    let status = upload(
        &world.app,
        &world.cookie,
        world.session_id,
        multipart_body("not_the_file", &TINY_PNG),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Bytes that are not an image at all are 415, distinct from the 413 an oversize image
/// gets. The frontend branches on the difference.
#[tokio::test]
async fn an_unsupported_type_is_refused_as_unsupported_media() {
    let world = upload_world(true, presigning_store()).await;

    let status = upload(
        &world.app,
        &world.cookie,
        world.session_id,
        multipart_body("file", b"not an image at all"),
    )
    .await;

    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

/// An unconfigured deployment is a deployment problem, not a caller error or a crash.
#[tokio::test]
async fn an_upload_without_object_storage_is_service_unavailable() {
    let world = upload_world(true, None).await;

    let status = upload(
        &world.app,
        &world.cookie,
        world.session_id,
        multipart_body("file", &TINY_PNG),
    )
    .await;

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn a_fetch_without_object_storage_is_service_unavailable() {
    let world = fetch_world(true, None).await;

    let status = fetch(&world.app, Some(&world.cookie), world.image_id)
        .await
        .status();

    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

/// A removal-signal world, plus the wire form of the row the domain update returns.
struct Signal {
    world: World,
    updated: serde_json::Value,
}

/// Builds a world for a removal signal. The last appended row is what the domain update
/// returns, so `returns_deleted` decides the `deleted_at` that update produced.
///
/// `deleted_at` is `#[serde(skip)]` on the model, so it cannot be asserted through the
/// response. The row the update returns is given a distinct `updated_at` instead and
/// compared whole: that is what fails if a handler stops delegating and simply echoes the
/// extractor's row back.
async fn signal_world(participant: bool, returns_deleted: bool) -> Signal {
    let organization_id = Id::new_v4();
    let user = coach();
    let role = role(user.id, organization_id);
    let coach_id = if participant { user.id } else { Id::new_v4() };
    let relationship = relationship(organization_id, coach_id, Id::new_v4());
    let session = session(relationship.id);
    // The stored row starts in the opposite state, so the update has something to change.
    let stored = image(session.id, coach_id, !returns_deleted);
    let updated = coaching_session_images::Model {
        deleted_at: returns_deleted.then(|| Utc::now().into()),
        updated_at: (Utc::now() + chrono::Duration::seconds(1)).into(),
        ..stored.clone()
    };
    let updated_wire = serde_json::to_value(&updated).expect("the model must serialize");

    let db = authenticated(&user, &role)
        .append_query_results([vec![stored.clone()]])
        .append_query_results([vec![(session.clone(), relationship.clone())]])
        .append_query_results([vec![updated]])
        .into_connection();

    let app = build_app(Arc::new(db), presigning_store());
    let cookie = login_cookie(&app).await;

    Signal {
        world: World {
            app,
            cookie,
            session_id: session.id,
            image_id: stored.id,
        },
        updated: updated_wire,
    }
}

/// Signals a removal (`DELETE`) or an undo (`POST .../restore`). Both are called by our own
/// API module over `sessionGuard`, so both send `x-version`.
async fn signal(
    app: &Router,
    cookie: Option<&str>,
    image_id: Id,
    method: &str,
    suffix: &str,
) -> axum::response::Response {
    let request = Request::builder()
        .uri(format!("/coaching_session_images/{image_id}{suffix}"))
        .method(method)
        .header("x-version", "1.0.0-beta1");
    let request = match cookie {
        Some(cookie) => request.header("cookie", cookie),
        None => request,
    };

    app.clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn soft_delete(app: &Router, cookie: Option<&str>, image_id: Id) -> axum::response::Response {
    signal(app, cookie, image_id, "DELETE", "").await
}

async fn undo(app: &Router, cookie: Option<&str>, image_id: Id) -> axum::response::Response {
    signal(app, cookie, image_id, "POST", "/restore").await
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).expect("the response must be JSON")
}

#[tokio::test]
async fn a_participant_marks_an_image_deleted() {
    let signal = signal_world(true, true).await;
    let world = &signal.world;

    let response = soft_delete(&world.app, Some(&world.cookie), world.image_id).await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert_eq!(body["status_code"], 200);
    assert_eq!(
        body["data"], signal.updated,
        "the response must carry the row the soft delete returned, not the extractor's"
    );
}

#[tokio::test]
async fn a_participant_restores_a_deleted_image() {
    let signal = signal_world(true, false).await;
    let world = &signal.world;

    let response = undo(&world.app, Some(&world.cookie), world.image_id).await;

    assert_eq!(response.status(), StatusCode::OK);

    let body = json_body(response).await;
    assert_eq!(body["status_code"], 200);
    assert_eq!(
        body["data"], signal.updated,
        "the response must carry the row the restore returned, not the extractor's"
    );
}

/// The headline case. Destruction is deferred precisely so an undo can resurrect the node
/// with the same id, which only works while the fetch keeps serving a marked row. This
/// fails the moment someone filters soft-deleted rows out of `read`.
#[tokio::test]
async fn a_fetch_still_serves_an_image_that_is_marked_deleted() {
    let world = fetch_world_for(true, presigning_store(), true).await;

    let status = fetch(&world.app, Some(&world.cookie), world.image_id)
        .await
        .status();

    assert!(
        status == StatusCode::FOUND || status == StatusCode::OK,
        "a soft-deleted image must still be served during the grace window, got {status}"
    );
}

#[tokio::test]
async fn a_non_participant_cannot_delete_an_image_it_holds_the_id_for() {
    let world = signal_world(false, true).await.world;

    let status = soft_delete(&world.app, Some(&world.cookie), world.image_id)
        .await
        .status();

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "a non-participant must be refused, got {status}"
    );
}

#[tokio::test]
async fn a_non_participant_cannot_restore_an_image_it_holds_the_id_for() {
    let world = signal_world(false, false).await.world;

    let status = undo(&world.app, Some(&world.cookie), world.image_id)
        .await
        .status();

    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "a non-participant must be refused, got {status}"
    );
}

#[tokio::test]
async fn an_unauthenticated_delete_is_unauthorized() {
    let world = signal_world(true, true).await.world;

    let status = soft_delete(&world.app, None, world.image_id).await.status();

    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
