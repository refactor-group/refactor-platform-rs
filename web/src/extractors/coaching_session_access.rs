use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
use domain::{coaching_session, Id};

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, lookup_rejection, RejectionType},
    AppState,
};
use domain::{coaching_relationships, coaching_sessions, users};
use log::*;

/// Extractor that verifies the authenticated user is a participant (coach or coachee)
/// in the coaching session identified by `coaching_session_id` in the URL path.
///
/// Works for any route containing `:coaching_session_id` regardless of other path params.
/// On success, yields the coaching session model so the handler can use it without an
/// additional database query.
pub(crate) struct CoachingSessionAccess(pub coaching_sessions::Model);

/// [`CoachingSessionAccess`] that also yields the session's coaching relationship, which the
/// access check loads anyway, for handlers that report what the relationship holds.
pub(crate) struct CoachingSessionContextAccess(
    pub coaching_sessions::Model,
    pub coaching_relationships::Model,
);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let (session, _) = authorize(parts, &AppState::from_ref(state)).await?;
        Ok(CoachingSessionAccess(session))
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionContextAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let (session, relationship) = authorize(parts, &AppState::from_ref(state)).await?;
        Ok(CoachingSessionContextAccess(session, relationship))
    }
}

/// Loads the session named in the path with its relationship, and admits the caller only if
/// the relationship grants them access.
async fn authorize(
    parts: &mut Parts,
    state: &AppState,
) -> Result<(coaching_sessions::Model, coaching_relationships::Model), RejectionType> {
    let AuthenticatedUser(authenticated_user) =
        AuthenticatedUser::from_request_parts(parts, state).await?;

    let Path(path_params) = Path::<HashMap<String, String>>::from_request_parts(parts, state)
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid path parameters".to_string(),
            )
        })?;

    let coaching_session_id: Id = path_params
        .get("coaching_session_id")
        .or_else(|| path_params.get("id"))
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing coaching_session_id in path".to_string(),
            )
        })?
        .parse()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid coaching session id".to_string(),
            )
        })?;

    authorize_session(state, &authenticated_user, coaching_session_id).await
}

/// Loads the session and its relationship, and admits `user` only if the relationship
/// grants them access.
pub(crate) async fn authorize_session(
    state: &AppState,
    user: &users::Model,
    coaching_session_id: Id,
) -> Result<(coaching_sessions::Model, coaching_relationships::Model), RejectionType> {
    debug!("Checking coaching session access for coaching_session_id={coaching_session_id}");

    let (coaching_session, coaching_relationship) =
        coaching_session::find_by_id_with_coaching_relationship(
            state.db_conn_ref(),
            coaching_session_id,
        )
        .await
        .map_err(lookup_rejection)?;

    if !coaching_relationship.grants_access_to(user) {
        return Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()));
    }

    Ok((coaching_session, coaching_relationship))
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use crate::middleware::auth::require_auth;

    use super::*;
    use crate::extractors::relationship_access_test_support::{self as support, Caller};
    use axum::{body::Body, middleware::from_fn};
    use axum::{extract::Request, routing::get, Router};
    use axum_login::{
        tower_sessions::{MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use chrono::Utc;
    use domain::user::Backend;
    use domain::user_roles;
    use domain::users;
    use password_auth::generate_hash;
    use sea_orm::{DatabaseBackend, DbErr, MockDatabase};
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
            working_hours_start: domain::working_hours::default_start(),
            working_hours_end: domain::working_hours::default_end(),
            roles: vec![],
            invite_status: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    async fn protected_route(
        CoachingSessionAccess(_coaching_session): CoachingSessionAccess,
    ) -> &'static str {
        "extracted_success"
    }

    #[tokio::test]
    async fn test_coaching_session_extractor_returns_404_when_coaching_session_exists_but_user_relationship_does_not(
    ) {
        // Create mock database with expected results
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let now = Utc::now();
        let test_user = create_test_user();

        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(Id::new_v4()),
            user_id: test_user.id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let test_session = coaching_sessions::Model {
            id: session_id,
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: None,
            ical_sequence: 0,
            ical_recurrence_id: None,
            collab_document_name: None,
            date: chrono::Utc::now().naive_utc(),
            duration_minutes: domain::duration::Duration::default_minutes(),
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: Some(now.into()),
            notice_given_at: chrono::Utc::now().into(),
        };

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results(vec![vec![(
                    test_session.clone(),
                    None::<coaching_relationships::Model>,
                )]])
                .into_connection(),
        );

        let app_state = AppState::new(
            service::AppState::new(Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default(),
            None,
            None,
            None,
        );

        // Set up session layer
        let session_store = MemoryStore::default();
        let session_layer = SessionManagerLayer::new(session_store)
            .with_secure(false)
            .with_expiry(Expiry::OnInactivity(Duration::days(1)))
            .with_always_save(true);

        let backend = Backend::new(&db);
        let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

        // Create app with login route and protected route
        let app = Router::new()
            .route(
                "/login",
                axum::routing::post(crate::controller::user_session_controller::login),
            )
            .merge(
                Router::new()
                    .route(
                        "/coaching_sessions/:coaching_session_id",
                        get(protected_route),
                    )
                    .route_layer(from_fn(require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state);

        // First, log in to create an authenticated session
        let login_request = Request::builder()
            .uri("/login")
            .method("POST")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("email=test@example.com&password=password123"))
            .unwrap();

        let login_response = app.clone().oneshot(login_request).await.unwrap();

        // Extract session cookie from login response
        let cookie = login_response
            .headers()
            .get("set-cookie")
            .and_then(|c| c.to_str().ok())
            .expect("Login should return session cookie");

        let protected_request = Request::builder()
            .uri(format!("/coaching_sessions/{}", session_id).as_str())
            .header("cookie", cookie)
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.clone().oneshot(protected_request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_failed_session_lookup_is_a_server_error() {
        let w = support::fixture();
        let db = Arc::new(
            w.signed_in(Caller::Coach)
                .append_query_errors([DbErr::Custom("connection lost".to_string())])
                .into_connection(),
        );
        let routes = Router::new().route(
            "/coaching_sessions/:coaching_session_id",
            get(protected_route),
        );

        let response = support::send(
            routes,
            &db,
            &w.coach.email,
            "GET",
            &format!("/coaching_sessions/{}", w.session.id),
            None,
        )
        .await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
