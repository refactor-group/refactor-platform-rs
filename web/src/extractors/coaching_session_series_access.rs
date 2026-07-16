use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path, Query},
    http::{request::Parts, StatusCode},
};
use domain::{
    coaching_relationship as CoachingRelationshipApi, coaching_relationships,
    coaching_session_series as CoachingSessionSeriesApi, Id,
};
use serde::Deserialize;

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, RejectionType},
    AppState,
};
use log::*;

/// Extracts a coaching session series and verifies the authenticated user is
/// a participant (coach OR coachee) of its parent relationship.
///
/// Used by read-only routes: `GET /coaching_session_series/:id`.
pub(crate) struct CoachingSessionSeriesAccess(pub CoachingSessionSeriesApi::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionSeriesAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let (series, relationship, user) = resolve(parts, state).await?;
        if relationship.coach_id == user.id || relationship.coachee_id == user.id {
            Ok(CoachingSessionSeriesAccess(series))
        } else {
            Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()))
        }
    }
}

/// Extracts the coaching relationship referenced by the `coaching_relationship_id`
/// query parameter and verifies the authenticated user is a participant
/// (coach OR coachee). Used by `GET /coaching_session_series` (list).
pub(crate) struct CoachingRelationshipQueryAccess(pub coaching_relationships::Model);

#[derive(Debug, Deserialize)]
struct RelationshipIdQuery {
    coaching_relationship_id: Id,
}

#[async_trait]
impl<S> FromRequestParts<S> for CoachingRelationshipQueryAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, &state).await?;

        let Query(params) = Query::<RelationshipIdQuery>::from_request_parts(parts, &state)
            .await
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Missing or invalid coaching_relationship_id query parameter".to_string(),
                )
            })?;

        let relationship = CoachingRelationshipApi::find_by_id(
            state.db_conn_ref(),
            params.coaching_relationship_id,
        )
        .await
        .map_err(|e| {
            error!(
                "Error finding coaching relationship {}: {e:?}",
                params.coaching_relationship_id
            );
            (StatusCode::NOT_FOUND, "NOT FOUND".to_string())
        })?;

        if relationship.coach_id == user.id || relationship.coachee_id == user.id {
            Ok(CoachingRelationshipQueryAccess(relationship))
        } else {
            Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()))
        }
    }
}

/// Coach-only variant. Used by write routes: `PUT` (reschedule) and `DELETE`
/// on `/coaching_session_series/:id`.
pub(crate) struct CoachingSessionSeriesCoachAccess(pub CoachingSessionSeriesApi::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionSeriesCoachAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let (series, relationship, user) = resolve(parts, state).await?;
        if relationship.coach_id == user.id {
            Ok(CoachingSessionSeriesCoachAccess(series))
        } else {
            Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()))
        }
    }
}

/// Shared lookup: extract `:id` from the path, load the series, load its
/// parent relationship, and pull the authenticated user. The caller decides
/// what membership rule to enforce.
async fn resolve<S>(
    parts: &mut Parts,
    state: &S,
) -> Result<
    (
        CoachingSessionSeriesApi::Model,
        coaching_relationships::Model,
        domain::users::Model,
    ),
    RejectionType,
>
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    let state = AppState::from_ref(state);
    let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, &state).await?;

    let Path(path_params) = Path::<HashMap<String, String>>::from_request_parts(parts, &state)
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid path parameters".to_string(),
            )
        })?;

    let series_id: Id = path_params
        .get("id")
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing series id in path".to_string(),
            )
        })?
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid series id".to_string()))?;

    debug!("Checking coaching_session_series access for series_id={series_id}");

    let series = CoachingSessionSeriesApi::find_by_id(state.db_conn_ref(), series_id)
        .await
        .map_err(|e| {
            error!("Error finding coaching_session_series {series_id}: {e:?}");
            (StatusCode::NOT_FOUND, "NOT FOUND".to_string())
        })?;

    let relationship =
        CoachingRelationshipApi::find_by_id(state.db_conn_ref(), series.coaching_relationship_id)
            .await
            .map_err(|e| {
                error!(
                    "Error finding coaching relationship {} for series {series_id}: {e:?}",
                    series.coaching_relationship_id
                );
                (StatusCode::NOT_FOUND, "NOT FOUND".to_string())
            })?;

    Ok((series, relationship, user))
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::middleware::auth::require_auth;
    use axum::{body::Body, extract::Request, middleware::from_fn, routing::get, Router};
    use axum_login::{
        tower_sessions::{MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use chrono::Utc;
    use domain::user::Backend;
    use domain::{coaching_relationships, user_roles, users};
    use password_auth::generate_hash;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use service::config::Config;
    use time::Duration;
    use tower::ServiceExt;
    use tower_sessions::Expiry;

    /// Builds a `users::Model` with the password hash `"password123"`.
    fn test_user() -> users::Model {
        let now = Utc::now();
        users::Model {
            id: Id::new_v4(),
            email: "test@example.com".to_string(),
            first_name: "Test".to_string(),
            last_name: "User".to_string(),
            display_name: None,
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

    fn test_relationship(coach_id: Id, coachee_id: Id) -> coaching_relationships::Model {
        let now = Utc::now();
        coaching_relationships::Model {
            id: Id::new_v4(),
            coach_id,
            coachee_id,
            organization_id: Id::new_v4(),
            slug: "test".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_series(
        coaching_relationship_id: Id,
        created_by_user_id: Id,
    ) -> CoachingSessionSeriesApi::Model {
        let now = Utc::now();
        CoachingSessionSeriesApi::Model {
            id: Id::new_v4(),
            coaching_relationship_id,
            rule: serde_json::json!({}),
            created_by_user_id,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    /// Build an app with `/login`, the given protected route, and full session
    /// + auth layers. Returns the app and a logged-in cookie for `user`.
    async fn build_app_and_login(
        db: Arc<sea_orm::DatabaseConnection>,
        route: &'static str,
        handler: axum::routing::MethodRouter<AppState>,
    ) -> (Router, String) {
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

        let app = Router::new()
            .route(
                "/login",
                axum::routing::post(crate::controller::user_session_controller::login),
            )
            .merge(
                Router::new()
                    .route(route, handler)
                    .route_layer(from_fn(require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state);

        let login_request = Request::builder()
            .uri("/login")
            .method("POST")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from("email=test@example.com&password=password123"))
            .unwrap();
        let login_response = app.clone().oneshot(login_request).await.unwrap();
        let cookie = login_response
            .headers()
            .get("set-cookie")
            .and_then(|c| c.to_str().ok())
            .expect("login should set a session cookie")
            .to_string();

        (app, cookie)
    }

    async fn series_access_route(
        CoachingSessionSeriesAccess(_series): CoachingSessionSeriesAccess,
    ) -> &'static str {
        "ok"
    }

    async fn series_coach_route(
        CoachingSessionSeriesCoachAccess(_series): CoachingSessionSeriesCoachAccess,
    ) -> &'static str {
        "ok"
    }

    async fn relationship_query_route(
        CoachingRelationshipQueryAccess(_relationship): CoachingRelationshipQueryAccess,
    ) -> &'static str {
        "ok"
    }

    /// Coachee opens GET /coaching_session_series/:id → coach-or-coachee gate
    /// lets them through.
    #[tokio::test]
    async fn series_access_allows_coachee() {
        let user = test_user();
        let role = test_role(user.id);
        let coach_id = Id::new_v4();
        let relationship = test_relationship(coach_id, user.id);
        let series = test_series(relationship.id, coach_id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                // Login: AuthN -> users + roles
                .append_query_results([vec![(user.clone(), role.clone())]])
                // require_auth: load again on the protected request
                .append_query_results([vec![(user.clone(), role.clone())]])
                // find_by_id: series row
                .append_query_results(vec![vec![series.clone()]])
                // Relationship lookup
                .append_query_results(vec![vec![relationship.clone()]])
                .into_connection(),
        );

        let (app, cookie) =
            build_app_and_login(db, "/coaching_session_series/:id", get(series_access_route)).await;

        let req = Request::builder()
            .uri(format!("/coaching_session_series/{}", series.id))
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Coachee hits the coach-only PUT/DELETE gate → 403.
    #[tokio::test]
    async fn coach_access_denies_coachee() {
        let user = test_user();
        let role = test_role(user.id);
        let coach_id = Id::new_v4();
        let relationship = test_relationship(coach_id, user.id);
        let series = test_series(relationship.id, coach_id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![series.clone()]])
                .append_query_results(vec![vec![relationship.clone()]])
                .into_connection(),
        );

        let (app, cookie) = build_app_and_login(
            db,
            "/coaching_session_series/:id",
            axum::routing::put(series_coach_route),
        )
        .await;

        let req = Request::builder()
            .uri(format!("/coaching_session_series/{}", series.id))
            .method("PUT")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    /// Coach hits the coach-only PUT/DELETE gate → 200.
    #[tokio::test]
    async fn coach_access_allows_coach() {
        let user = test_user();
        let role = test_role(user.id);
        let coachee_id = Id::new_v4();
        let relationship = test_relationship(user.id, coachee_id);
        let series = test_series(relationship.id, user.id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![series.clone()]])
                .append_query_results(vec![vec![relationship.clone()]])
                .into_connection(),
        );

        let (app, cookie) = build_app_and_login(
            db,
            "/coaching_session_series/:id",
            axum::routing::put(series_coach_route),
        )
        .await;

        let req = Request::builder()
            .uri(format!("/coaching_session_series/{}", series.id))
            .method("PUT")
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Coach hits GET /coaching_session_series?coaching_relationship_id=... → 200.
    #[tokio::test]
    async fn relationship_query_access_allows_coach() {
        let user = test_user();
        let role = test_role(user.id);
        let coachee_id = Id::new_v4();
        let relationship = test_relationship(user.id, coachee_id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![relationship.clone()]])
                .into_connection(),
        );

        let (app, cookie) = build_app_and_login(
            db,
            "/coaching_session_series",
            get(relationship_query_route),
        )
        .await;

        let req = Request::builder()
            .uri(format!(
                "/coaching_session_series?coaching_relationship_id={}",
                relationship.id
            ))
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    /// Outsider (neither coach nor coachee) on the index gate → 403.
    #[tokio::test]
    async fn relationship_query_access_denies_outsider() {
        let user = test_user();
        let role = test_role(user.id);
        let relationship = test_relationship(Id::new_v4(), Id::new_v4());

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results([vec![(user.clone(), role.clone())]])
                .append_query_results(vec![vec![relationship.clone()]])
                .into_connection(),
        );

        let (app, cookie) = build_app_and_login(
            db,
            "/coaching_session_series",
            get(relationship_query_route),
        )
        .await;

        let req = Request::builder()
            .uri(format!(
                "/coaching_session_series?coaching_relationship_id={}",
                relationship.id
            ))
            .header("cookie", cookie)
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
