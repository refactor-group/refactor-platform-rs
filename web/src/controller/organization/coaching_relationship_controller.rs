use crate::controller::ApiResponse;
use crate::extractors::organization_member_access::OrganizationMemberAccess;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_relationship::action::{AssigneeFilter, IndexParams};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::coaching_relationship::CoachingRelationshipWithUserNames;
use domain::{
    action as ActionApi, coaching_relationship as CoachingRelationshipApi, coaching_relationships,
    goal_progress as GoalProgressApi, Id, QuerySort,
};
use service::config::ApiVersion;

use log::*;

/// CREATE a new CoachingRelationship.
#[utoipa::path(
    post,
    path = "/organizations/{organization_id}/coaching_relationships",
    params(
        ApiVersion,
    ),
    request_body = entity::coaching_relationships::Model,
    responses(
        (status = 200, description = "Successfully created a new Coaching Relationship", body = [coaching_relationships::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
    )]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    OrganizationMemberAccess(organization_id): OrganizationMemberAccess,
    Json(coaching_relationship_model): Json<coaching_relationships::Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("CREATE new Coaching Relationship from: {coaching_relationship_model:?}");

    let coaching_relationship: CoachingRelationshipWithUserNames = CoachingRelationshipApi::create(
        app_state.db_conn_ref(),
        organization_id,
        coaching_relationship_model,
    )
    .await?;

    debug!(
        "Newly created Coaching Relationship: {:?}",
        &coaching_relationship
    );

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        coaching_relationship,
    )))
}

/// GET a particular CoachingRelationship specified by the organization Id and relationship Id.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/{relationship_id}",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "Organization id to retrieve the CoachingRelationship under"),
        ("relationship_id" = String, Path, description = "CoachingRelationship id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a certain CoachingRelationship by its id", body = [coaching_relationships::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "CoachingRelationship not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Path((_organization_id, relationship_id)): Path<(Id, Id)>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET CoachingRelationship by id: {relationship_id}");

    let relationship: Option<CoachingRelationshipWithUserNames> =
        CoachingRelationshipApi::get_relationship_with_user_names(
            app_state.db_conn_ref(),
            relationship_id,
        )
        .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), relationship)))
}

/// GET all CoachingRelationships by organization_id
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "Organization id to retrieve CoachingRelationships")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all CoachingRelationships", body = [coaching_relationships::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    OrganizationMemberAccess(organization_id): OrganizationMemberAccess,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "GET all CoachingRelationships for user {} in organization {}",
        user.id, organization_id
    );

    let coaching_relationships =
        CoachingRelationshipApi::find_by_organization_for_user_with_user_names(
            app_state.db_conn_ref(),
            user.id,
            organization_id,
        )
        .await?;

    debug!("Found CoachingRelationships: {coaching_relationships:?}");

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        coaching_relationships,
    )))
}

/// GET aggregate goal progress for all goals in a coaching relationship.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/{relationship_id}/goal_progress",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "Organization id"),
        ("relationship_id" = Id, Path, description = "Coaching relationship id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved goal progress for the coaching relationship"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn goal_progress(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path((_organization_id, relationship_id)): Path<(Id, Id)>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET goal progress for coaching relationship: {relationship_id}");

    let progress =
        GoalProgressApi::relationship_goal_progress(app_state.db_conn_ref(), relationship_id)
            .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), progress)))
}

/// GET actions for a specific coaching relationship.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/{relationship_id}/actions",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "Organization id"),
        ("relationship_id" = Id, Path, description = "Coaching relationship id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved actions for the coaching relationship"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn actions(
    CompareApiVersion(_v): CompareApiVersion,
    OrganizationMemberAccess(_organization_id): OrganizationMemberAccess,
    State(app_state): State<AppState>,
    Path((_org_id, relationship_id)): Path<(Id, Id)>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET actions for coaching relationship: {relationship_id}");

    let params = params.apply_defaults();

    let sort_column = params.get_sort_column();
    let sort_order = params.get_sort_order();

    let query_params = ActionApi::FindByRelationshipParams {
        status: params.status,
        assignee_filter: match params.assignee_filter {
            AssigneeFilter::All => ActionApi::AssigneeFilter::All,
            AssigneeFilter::Assigned => ActionApi::AssigneeFilter::Assigned,
            AssigneeFilter::Unassigned => ActionApi::AssigneeFilter::Unassigned,
        },
        sort_column,
        sort_order,
    };

    let actions = ActionApi::find_by_coaching_relationship(
        app_state.db_conn_ref(),
        relationship_id,
        query_params,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), actions)))
}

/// GET actions for all coachees of the authenticated coach, grouped by coachee user ID.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/coachee-actions",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "Organization id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved batch coachee actions"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn batch_coachee_actions(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    OrganizationMemberAccess(organization_id): OrganizationMemberAccess,
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "GET batch coachee actions for coach {} in organization {}",
        user.id, organization_id
    );

    let params = params.apply_defaults();

    let sort_column = params.get_sort_column();
    let sort_order = params.get_sort_order();

    let query_params = ActionApi::FindByRelationshipParams {
        status: params.status,
        assignee_filter: match params.assignee_filter {
            AssigneeFilter::All => ActionApi::AssigneeFilter::All,
            AssigneeFilter::Assigned => ActionApi::AssigneeFilter::Assigned,
            AssigneeFilter::Unassigned => ActionApi::AssigneeFilter::Unassigned,
        },
        sort_column,
        sort_order,
    };

    let relationships = CoachingRelationshipApi::find_by_coach_and_organization(
        app_state.db_conn_ref(),
        user.id,
        organization_id,
    )
    .await?;

    let coachee_actions = ActionApi::find_by_coach_relationships(
        app_state.db_conn_ref(),
        &relationships,
        query_params,
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        serde_json::json!({ "coachee_actions": coachee_actions }),
    )))
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::extract::Request;
    use axum::middleware::from_fn;
    use axum::routing::get;
    use axum::Router;
    use axum_login::tower_sessions::{MemoryStore, SessionManagerLayer};
    use axum_login::AuthManagerLayerBuilder;
    use chrono::Utc;
    use domain::user::Backend;
    use domain::{user_roles, users, Id};
    use password_auth::generate_hash;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use time::Duration;
    use tower::ServiceExt;
    use tower_sessions::Expiry;

    use crate::middleware::auth::require_auth;
    use crate::protect;
    use crate::AppState;

    fn create_test_user() -> users::Model {
        let now = Utc::now();
        users::Model {
            id: Id::new_v4(),
            email: "test@example.com".to_string(),
            first_name: "Test".to_string(),
            last_name: "User".to_string(),
            display_name: Some("Test User".to_string()),
            password: generate_hash("password123".to_string()),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            role: users::Role::User,
            roles: vec![],
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    /// Helper to build a test app with auth layers and our actual routes.
    fn build_test_app(db: Arc<sea_orm::DatabaseConnection>) -> Router {
        let app_state = AppState::new(
            service::AppState::new(service::config::Config::default(), &db),
            Arc::new(sse::Manager::default()),
            domain::events::EventPublisher::default(),
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
                        "/organizations/:organization_id/coaching_relationships/coachee-actions",
                        get(super::batch_coachee_actions),
                    )
                    .merge(
                        Router::new()
                            .route(
                                "/organizations/:organization_id/coaching_relationships/:relationship_id/actions",
                                get(super::actions),
                            )
                            .route_layer(axum::middleware::from_fn_with_state(
                                app_state.clone(),
                                protect::organizations::coaching_relationships::actions,
                            )),
                    )
                    .route_layer(from_fn(require_auth)),
            )
            .layer(auth_layer)
            .with_state(app_state)
    }

    /// Helper to log in and return the session cookie.
    async fn login(app: &Router) -> String {
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
            .and_then(|c| c.to_str().ok())
            .expect("Login should return session cookie")
            .to_string()
    }

    /// A user who is NOT a member of the organization should get 401
    /// from the batch coachee-actions endpoint — not an empty 200.
    /// This proves OrganizationMemberAccess is wired up on the batch endpoint,
    /// preventing org ID probing via empty responses.
    #[tokio::test]
    async fn batch_coachee_actions_rejects_non_org_member() {
        let organization_id = Id::new_v4();
        let other_user_id = Id::new_v4();
        let now = Utc::now();
        let test_user = create_test_user();

        // Role with organization_id = None → user is NOT a member of any org
        let test_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: None,
            user_id: test_user.id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        // A different user who IS in the org (returned by find_by_organization)
        let org_member = users::Model {
            id: other_user_id,
            email: "other@example.com".to_string(),
            first_name: "Other".to_string(),
            last_name: "User".to_string(),
            display_name: None,
            password: generate_hash("password456".to_string()),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            role: users::Role::User,
            roles: vec![],
            created_at: now.into(),
            updated_at: now.into(),
        };
        let org_member_role = user_roles::Model {
            id: Id::new_v4(),
            role: users::Role::User,
            organization_id: Some(organization_id),
            user_id: other_user_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        // Mock DB query sequence (FIFO):
        // 1) Login authenticate: find_by_email (user + role join)
        // 2) Login get_user: find_by_id + roles
        // 3) Protected request require_auth: get_user (session lookup)
        // 4) AuthenticatedUser extractor (inside OrganizationMemberAccess)
        // 5) OrganizationMemberAccess: find_by_organization → different user
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                .append_query_results([vec![(test_user.clone(), test_role.clone())]])
                // find_by_organization: returns a DIFFERENT user → test_user not found → 401
                .append_query_results([vec![(org_member.clone(), org_member_role.clone())]])
                .into_connection(),
        );

        let app = build_test_app(db);
        let cookie = login(&app).await;

        let request = Request::builder()
            .uri(format!(
                "/organizations/{}/coaching_relationships/coachee-actions",
                organization_id
            ))
            .header("cookie", &cookie)
            .header("x-version", "1.0.0-beta1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "Non-org-member should be rejected with 401, not get an empty 200"
        );
    }

    // The single-relationship actions endpoint's protect middleware is tested
    // directly in protect/organizations/coaching_relationships.rs with two tests:
    // - actions_middleware_rejects_non_participant (401 for non-participant)
    // - actions_middleware_allows_coach (200 for coach)
    //
    // OrganizationMemberAccess on the handler is the same extractor tested by
    // batch_coachee_actions_rejects_non_org_member above.
}
