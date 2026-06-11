use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
use domain::{coaching_session, coaching_session_topic, coaching_session_topics, Id};

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser, coaching_session_access::CoachingSessionAccess,
        RejectionType,
    },
    AppState,
};

/// Verifies the authenticated user is a participant of the path session AND that the
/// `:topic_id` topic belongs to that session.
///
/// Composes `CoachingSessionAccess` (participant + session check), then loads the topic
/// and confirms it belongs to the path session. Any failure collapses to 404 so a topic
/// in an inaccessible session is never revealed. On success, yields the topic model.
pub(crate) struct CoachingSessionTopicAccess(pub coaching_session_topics::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionTopicAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        // Composes the participant + session check (reuses the tested extractor).
        let CoachingSessionAccess(session) =
            CoachingSessionAccess::from_request_parts(parts, state).await?;

        let Path(path_params) =
            Path::<HashMap<String, String>>::from_request_parts(parts, &app_state)
                .await
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "Invalid path parameters".to_string(),
                    )
                })?;

        let topic_id: Id = path_params
            .get("topic_id")
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing topic_id in path".to_string(),
            ))?
            .parse()
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid topic id".to_string()))?;

        // Load + verify the topic belongs to THIS session (else 404 to hide existence).
        let topic = coaching_session_topic::find_by_id(app_state.db_conn_ref(), topic_id)
            .await
            .map_err(|_| (StatusCode::NOT_FOUND, "NOT FOUND".to_string()))?;

        if topic.coaching_session_id != session.id {
            return Err((StatusCode::NOT_FOUND, "NOT FOUND".to_string()));
        }

        Ok(CoachingSessionTopicAccess(topic))
    }
}

/// Authorizes a topic delete. Composes `CoachingSessionTopicAccess` (participant + topic belongs
/// to the path session), then allows the caller only if they are the topic's author OR the coach
/// of the session's relationship. So a coach may delete any topic in the session (including a
/// coachee's), while a coachee may delete only their own. Any failure collapses to 404.
pub(crate) struct CoachingSessionTopicDeleteAccess(pub coaching_session_topics::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionTopicDeleteAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let CoachingSessionTopicAccess(topic) =
            CoachingSessionTopicAccess::from_request_parts(parts, state).await?;

        let AuthenticatedUser(user) =
            AuthenticatedUser::from_request_parts(parts, &app_state).await?;

        // The author may delete their own topic.
        if topic.user_id == user.id {
            return Ok(CoachingSessionTopicDeleteAccess(topic));
        }

        // Otherwise only the coach of the session's relationship may delete it.
        let (_session, relationship) = coaching_session::find_by_id_with_coaching_relationship(
            app_state.db_conn_ref(),
            topic.coaching_session_id,
        )
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "NOT FOUND".to_string()))?;

        if relationship.coach_id == user.id {
            return Ok(CoachingSessionTopicDeleteAccess(topic));
        }

        Err((StatusCode::NOT_FOUND, "NOT FOUND".to_string()))
    }
}

/// Authorizes an undo. Composes CoachingSessionAccess (participant + path session), loads the
/// topic INCLUDING soft-deleted, and confirms it belongs to the path session. Undoing a delete
/// (the topic is soft-deleted) additionally requires the caller to be the author. Any failure
/// collapses to 404 so an inaccessible topic is never revealed.
pub(crate) struct CoachingSessionTopicUndoAccess(pub coaching_session_topics::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionTopicUndoAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let CoachingSessionAccess(session) =
            CoachingSessionAccess::from_request_parts(parts, state).await?;

        let Path(path_params) =
            Path::<HashMap<String, String>>::from_request_parts(parts, &app_state)
                .await
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "Invalid path parameters".to_string(),
                    )
                })?;

        let topic_id: Id = path_params
            .get("topic_id")
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing topic_id in path".to_string(),
            ))?
            .parse()
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid topic id".to_string()))?;

        let topic =
            coaching_session_topic::find_including_deleted_by_id(app_state.db_conn_ref(), topic_id)
                .await
                .map_err(|_| (StatusCode::NOT_FOUND, "NOT FOUND".to_string()))?;

        if topic.coaching_session_id != session.id {
            return Err((StatusCode::NOT_FOUND, "NOT FOUND".to_string()));
        }

        // Undoing a delete is author-only; undoing a defer is open to either participant.
        if topic.deleted_at.is_some() {
            let AuthenticatedUser(user) =
                AuthenticatedUser::from_request_parts(parts, &app_state).await?;
            if topic.user_id != user.id {
                return Err((StatusCode::NOT_FOUND, "NOT FOUND".to_string()));
            }
        }

        Ok(CoachingSessionTopicUndoAccess(topic))
    }
}

/// Rating writes are coachee-only. Verifies the caller is the coachee of the path session's
/// relationship (else 403), and that the `:topic_id` topic belongs to that session (else 404).
pub(crate) struct CoachingSessionTopicCoacheeAccess(pub coaching_session_topics::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionTopicCoacheeAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let AuthenticatedUser(user) =
            AuthenticatedUser::from_request_parts(parts, &app_state).await?;

        let Path(path_params) =
            Path::<HashMap<String, String>>::from_request_parts(parts, &app_state)
                .await
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "Invalid path parameters".to_string(),
                    )
                })?;

        let coaching_session_id: Id = path_params
            .get("coaching_session_id")
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing coaching_session_id in path".to_string(),
            ))?
            .parse()
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Invalid coaching session id".to_string(),
                )
            })?;

        let topic_id: Id = path_params
            .get("topic_id")
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing topic_id in path".to_string(),
            ))?
            .parse()
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid topic id".to_string()))?;

        let (session, relationship) = coaching_session::find_by_id_with_coaching_relationship(
            app_state.db_conn_ref(),
            coaching_session_id,
        )
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "NOT FOUND".to_string()))?;

        // Coachee-only: a coach can read the topic but not rate it -> 403.
        if relationship.coachee_id != user.id {
            return Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()));
        }

        let topic = coaching_session_topic::find_by_id(app_state.db_conn_ref(), topic_id)
            .await
            .map_err(|_| (StatusCode::NOT_FOUND, "NOT FOUND".to_string()))?;

        if topic.coaching_session_id != session.id {
            return Err((StatusCode::NOT_FOUND, "NOT FOUND".to_string()));
        }

        Ok(CoachingSessionTopicCoacheeAccess(topic))
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use crate::middleware::auth::require_auth;

    use super::*;
    use axum::{body::Body, middleware::from_fn};
    use axum::{
        extract::Request,
        routing::{delete, patch, post},
        Router,
    };
    use axum_login::{
        tower_sessions::{MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use chrono::Utc;
    use domain::user::Backend;
    use domain::users;
    use domain::{coaching_relationships, coaching_sessions, user_roles};
    use password_auth::generate_hash;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use service::config::Config;
    use time::Duration;
    use tower::ServiceExt;
    use tower_sessions::Expiry;

    fn create_test_user() -> users::Model {
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
            default_coaching_session_duration_minutes: domain::duration::Duration::default_minutes(
            ),
            role: users::Role::User,
            roles: vec![],
            invite_status: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_role(user_id: Id) -> user_roles::Model {
        let now = Utc::now();
        user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(Id::new_v4()),
            user_id,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_session(session_id: Id, relationship_id: Id) -> coaching_sessions::Model {
        let now = Utc::now();
        coaching_sessions::Model {
            id: session_id,
            coaching_relationship_id: relationship_id,
            collab_document_name: None,
            date: now.naive_utc(),
            duration_minutes: domain::duration::Duration::default_minutes(),
            meeting_url: None,
            provider: None,
            hydrated_at: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_relationship(relationship_id: Id, coachee_id: Id) -> coaching_relationships::Model {
        let now = Utc::now();
        coaching_relationships::Model {
            id: relationship_id,
            coach_id: Id::new_v4(),
            coachee_id,
            organization_id: Id::new_v4(),
            slug: "test".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_relationship_with_coach(
        relationship_id: Id,
        coach_id: Id,
        coachee_id: Id,
    ) -> coaching_relationships::Model {
        let now = Utc::now();
        coaching_relationships::Model {
            id: relationship_id,
            coach_id,
            coachee_id,
            organization_id: Id::new_v4(),
            slug: "test".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_topic(
        topic_id: Id,
        coaching_session_id: Id,
        user_id: Id,
    ) -> coaching_session_topics::Model {
        let now = Utc::now();
        coaching_session_topics::Model {
            id: topic_id,
            coaching_session_id,
            body: "A topic".to_string(),
            user_id,
            display_order: 0,
            priority: Some(domain::topic_priority::Priority::High),
            status: domain::topic_status::Status::Open,
            moved_from_session_id: None,
            undo_snapshot: None,
            deleted_at: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    // Mounts the delete extractor behind require_auth so a logged-in DELETE exercises the full
    // composed chain (session participant -> topic-belongs-to-session -> author-or-coach).
    async fn delete_route(
        CoachingSessionTopicDeleteAccess(_topic): CoachingSessionTopicDeleteAccess,
    ) -> &'static str {
        "extracted_success"
    }

    // Mounts the coachee extractor behind require_auth so a logged-in PATCH exercises the
    // coachee-only rating guard (coachee -> ok, coach -> 403).
    async fn coachee_route(
        CoachingSessionTopicCoacheeAccess(_topic): CoachingSessionTopicCoacheeAccess,
    ) -> &'static str {
        "extracted_success"
    }

    // Mounts the undo extractor behind require_auth so a logged-in POST exercises the
    // state-derived undo guard (defer: any participant; delete: author only).
    async fn undo_route(
        CoachingSessionTopicUndoAccess(_topic): CoachingSessionTopicUndoAccess,
    ) -> &'static str {
        "extracted_success"
    }

    fn build_app(db: Arc<sea_orm::DatabaseConnection>) -> Router {
        let app_state = AppState::new(
            service::AppState::new(Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default(),
            None,
            None,
        );

        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::days(1)))
            .with_always_save(true);

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        Router::new()
            .route(
                "/login",
                axum::routing::post(crate::controller::user_session_controller::login),
            )
            .merge(
                Router::new()
                    .route(
                        "/coaching_sessions/:coaching_session_id/topics/:topic_id",
                        delete(delete_route),
                    )
                    .route(
                        "/coaching_sessions/:coaching_session_id/topics/:topic_id/rating",
                        patch(coachee_route),
                    )
                    .route(
                        "/coaching_sessions/:coaching_session_id/topics/:topic_id/undo",
                        post(undo_route),
                    )
                    .route_layer(from_fn(require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state)
    }

    async fn do_login(app: &Router) -> String {
        let req = Request::builder()
            .uri("/login")
            .method("POST")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("email=test@example.com&password=password123"))
            .unwrap();

        let resp = app.clone().oneshot(req).await.unwrap();
        resp.headers()
            .get("set-cookie")
            .and_then(|c| c.to_str().ok())
            .expect("Login should return session cookie")
            .to_string()
    }

    #[tokio::test]
    async fn delete_extractor_ok_when_author() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let topic_id = Id::new_v4();
        let user = create_test_user();
        let role = test_role(user.id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    test_relationship(relationship_id, user.id),
                )]])
                .append_query_results(vec![vec![test_topic(topic_id, session_id, user.id)]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app).await;

        let req = Request::builder()
            .uri(format!("/coaching_sessions/{session_id}/topics/{topic_id}"))
            .method("DELETE")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // A coachee deleting the coach's topic: not the author, not the coach -> 404.
    #[tokio::test]
    async fn delete_extractor_404_when_coachee_deletes_coachs_topic() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let topic_id = Id::new_v4();
        let coach_id = Id::new_v4();
        let user = create_test_user(); // caller acts as the coachee
        let role = test_role(user.id);
        let relationship = test_relationship_with_coach(relationship_id, coach_id, user.id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    relationship.clone(),
                )]])
                // Topic authored by the coach -> caller is not the author.
                .append_query_results(vec![vec![test_topic(topic_id, session_id, coach_id)]])
                // Coach check: caller is the coachee, not the coach -> 404.
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    relationship.clone(),
                )]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app).await;

        let req = Request::builder()
            .uri(format!("/coaching_sessions/{session_id}/topics/{topic_id}"))
            .method("DELETE")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // A coach deleting the coachee's topic: not the author, but is the coach -> allowed.
    #[tokio::test]
    async fn delete_extractor_ok_when_coach_deletes_coachees_topic() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let topic_id = Id::new_v4();
        let coachee_id = Id::new_v4();
        let user = create_test_user(); // caller acts as the coach
        let role = test_role(user.id);
        let relationship = test_relationship_with_coach(relationship_id, user.id, coachee_id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    relationship.clone(),
                )]])
                // Topic authored by the coachee -> caller (coach) is not the author.
                .append_query_results(vec![vec![test_topic(topic_id, session_id, coachee_id)]])
                // Coach check: caller IS the coach of the relationship -> allowed.
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    relationship.clone(),
                )]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app).await;

        let req = Request::builder()
            .uri(format!("/coaching_sessions/{session_id}/topics/{topic_id}"))
            .method("DELETE")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn delete_extractor_404_when_topic_belongs_to_other_session() {
        let session_id = Id::new_v4();
        let other_session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let topic_id = Id::new_v4();
        let user = create_test_user();
        let role = test_role(user.id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    test_relationship(relationship_id, user.id),
                )]])
                // Topic belongs to a different session -> session-match guard fails to 404.
                .append_query_results(vec![vec![test_topic(topic_id, other_session_id, user.id)]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app).await;

        let req = Request::builder()
            .uri(format!("/coaching_sessions/{session_id}/topics/{topic_id}"))
            .method("DELETE")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn coachee_extractor_ok_when_user_is_coachee() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let topic_id = Id::new_v4();
        let user = create_test_user();
        let role = test_role(user.id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                // Caller is the coachee of the relationship -> rating allowed.
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    test_relationship(relationship_id, user.id),
                )]])
                .append_query_results(vec![vec![test_topic(topic_id, session_id, user.id)]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app).await;

        let req = Request::builder()
            .uri(format!(
                "/coaching_sessions/{session_id}/topics/{topic_id}/rating"
            ))
            .method("PATCH")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn coachee_extractor_403_when_user_is_coach() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let topic_id = Id::new_v4();
        let coachee_id = Id::new_v4();
        let user = create_test_user();
        let role = test_role(user.id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                // Caller is the coach (a participant) but not the coachee -> 403.
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    test_relationship_with_coach(relationship_id, user.id, coachee_id),
                )]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app).await;

        let req = Request::builder()
            .uri(format!(
                "/coaching_sessions/{session_id}/topics/{topic_id}/rating"
            ))
            .method("PATCH")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    /// Builds a soft-deleted topic (deleted_at set) authored by `user_id`.
    fn deleted_test_topic(
        topic_id: Id,
        coaching_session_id: Id,
        user_id: Id,
    ) -> coaching_session_topics::Model {
        let now = Utc::now();
        coaching_session_topics::Model {
            deleted_at: Some(now.into()),
            ..test_topic(topic_id, coaching_session_id, user_id)
        }
    }

    // Undoing a defer is open to either participant: a live (not soft-deleted) topic
    // authored by someone else is still undoable by the caller -> 200.
    #[tokio::test]
    async fn undo_extractor_ok_for_live_topic_by_non_author() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let topic_id = Id::new_v4();
        let other_user_id = Id::new_v4();
        let user = create_test_user();
        let role = test_role(user.id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    test_relationship(relationship_id, user.id),
                )]])
                // Live topic authored by someone else: defer-undo needs no author check.
                .append_query_results(vec![vec![test_topic(topic_id, session_id, other_user_id)]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app).await;

        let req = Request::builder()
            .uri(format!(
                "/coaching_sessions/{session_id}/topics/{topic_id}/undo"
            ))
            .method("POST")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // Undoing a delete is author-only: the soft-deleted topic's author -> 200.
    #[tokio::test]
    async fn undo_extractor_ok_for_deleted_topic_by_author() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let topic_id = Id::new_v4();
        let user = create_test_user();
        let role = test_role(user.id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    test_relationship(relationship_id, user.id),
                )]])
                // Soft-deleted topic authored by the caller -> author branch passes.
                .append_query_results(vec![vec![deleted_test_topic(
                    topic_id, session_id, user.id,
                )]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app).await;

        let req = Request::builder()
            .uri(format!(
                "/coaching_sessions/{session_id}/topics/{topic_id}/undo"
            ))
            .method("POST")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // Security teeth: a soft-deleted topic authored by someone else is NOT undoable by a
    // non-author participant -> 404. The author-only branch fires only for deleted topics.
    #[tokio::test]
    async fn undo_extractor_404_for_deleted_topic_by_non_author() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let topic_id = Id::new_v4();
        let other_user_id = Id::new_v4();
        let user = create_test_user();
        let role = test_role(user.id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    test_relationship(relationship_id, user.id),
                )]])
                // Soft-deleted topic authored by someone else -> author guard fails to 404.
                .append_query_results(vec![vec![deleted_test_topic(
                    topic_id,
                    session_id,
                    other_user_id,
                )]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app).await;

        let req = Request::builder()
            .uri(format!(
                "/coaching_sessions/{session_id}/topics/{topic_id}/undo"
            ))
            .method("POST")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // A topic that belongs to a different session -> 404 (session-match guard).
    #[tokio::test]
    async fn undo_extractor_404_when_topic_belongs_to_other_session() {
        let session_id = Id::new_v4();
        let other_session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let topic_id = Id::new_v4();
        let user = create_test_user();
        let role = test_role(user.id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![(
                    test_session(session_id, relationship_id),
                    test_relationship(relationship_id, user.id),
                )]])
                // Topic belongs to a different session -> session-match guard fails to 404.
                .append_query_results(vec![vec![test_topic(topic_id, other_session_id, user.id)]])
                .into_connection(),
        );

        let app = build_app(Arc::clone(&db));
        let cookie = do_login(&app).await;

        let req = Request::builder()
            .uri(format!(
                "/coaching_sessions/{session_id}/topics/{topic_id}/undo"
            ))
            .method("POST")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
