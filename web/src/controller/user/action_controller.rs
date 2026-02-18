use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::user::action::{AssigneeFilter, IndexParams, Scope};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{action as ActionApi, Id, QuerySort};
use service::config::ApiVersion;

use log::*;

/// GET actions for a user with flexible filtering
///
/// This unified endpoint supports two query scopes:
/// - `scope=sessions` (default): All actions from coaching sessions where user is coach or coachee
/// - `scope=assigned`: Actions where the user is an assignee
#[utoipa::path(
    get,
    path = "/users/{user_id}/actions",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve actions for"),
        ("scope" = Option<String>, Query, description = "Scope: 'sessions' (default) or 'assigned'"),
        ("coaching_session_id" = Option<Id>, Query, description = "Filter by coaching session"),
        ("coaching_relationship_id" = Option<Id>, Query, description = "Filter by coaching relationship"),
        ("assignee_filter" = Option<String>, Query, description = "Filter: 'all' (default), 'assigned', or 'unassigned'"),
        ("status" = Option<String>, Query, description = "Filter by action status"),
        ("sort_by" = Option<String>, Query, description = "Sort by: 'due_by', 'created_at', 'updated_at'"),
        ("sort_order" = Option<String>, Query, description = "Sort order: 'asc' or 'desc'")
    ),
    responses(
        (status = 200, description = "Successfully retrieved actions for user", body = [domain::action::ActionWithAssignees]),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "User not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Actions for User: {user_id}");
    debug!("Params: {params:?}");

    // Set user_id from path and apply defaults
    let params = params.with_user_id(user_id).apply_defaults();

    // Extract sort options before partial moves
    let sort_column = params.get_sort_column();
    let sort_order = params.get_sort_order();

    // Map web layer types to domain layer types
    let query_params = ActionApi::FindByUserParams {
        scope: match params.scope {
            Scope::Assigned => ActionApi::Scope::Assigned,
            Scope::Sessions => ActionApi::Scope::Sessions,
        },
        coaching_session_id: params.coaching_session_id,
        coaching_relationship_id: params.coaching_relationship_id,
        status: params.status,
        assignee_filter: match params.assignee_filter {
            AssigneeFilter::All => ActionApi::AssigneeFilter::All,
            AssigneeFilter::Assigned => ActionApi::AssigneeFilter::Assigned,
            AssigneeFilter::Unassigned => ActionApi::AssigneeFilter::Unassigned,
        },
        sort_column,
        sort_order,
    };

    let actions = ActionApi::find_by_user(app_state.db_conn_ref(), user_id, query_params).await?;

    debug!("Found {} actions for user {user_id}", actions.len());

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), actions)))
}
