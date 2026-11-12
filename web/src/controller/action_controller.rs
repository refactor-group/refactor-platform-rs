use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use domain::action::ActionWithAssignees;
use domain::{
    action as ActionApi, actions::Model, coaching_relationship as CoachingRelationshipApi,
    coaching_relationships, emails as EmailsApi, goal as GoalApi, users, Id,
};
use log::*;
use sea_orm::DatabaseConnection;
use serde::Deserialize;
use serde_json::json;
use service::config::ApiVersion;
use utoipa::ToSchema;

use crate::controller::ApiResponse;
use crate::extractors::{
    action_access::ActionAccess, authenticated_user::AuthenticatedUser,
    coaching_session_access::authorize_session, compare_api_version::CompareApiVersion,
};
use crate::params::action::{IndexParams, SortField};
use crate::params::status;
use crate::params::WithSortDefaults;
use crate::{AppState, Error};

/// Request body for creating or updating an action.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ActionRequest {
    #[serde(flatten)]
    pub action: Model,
    /// Optional list of user IDs to assign to this action.
    /// For updates, if provided, replaces all existing assignees.
    /// If omitted during update, assignees remain unchanged.
    pub assignee_ids: Option<Vec<Id>>,
}

impl ActionRequest {
    /// Returns true if the request explicitly includes an `assignee_ids` field.
    /// Note: this only signals that the assignee list should be *replaced*, not
    /// that notifications should be sent — the caller must diff against the
    /// previous assignee set to determine which users (if any) are newly added.
    pub fn have_assignees_changed(&self) -> bool {
        self.assignee_ids.is_some()
    }
}

/// Refuses assignees who are not the relationship's coach or coachee, an added assignee no
/// longer in its organization, and a goal from another relationship, before anything is
/// written. `previous` are the action's current assignees, which are not re-checked.
async fn validate_links(
    db: &DatabaseConnection,
    relationship: &coaching_relationships::Model,
    request: &ActionRequest,
    previous: &[Id],
) -> Result<(), Error> {
    let assignees_in_relationship = request
        .assignee_ids
        .iter()
        .flatten()
        .all(|id| relationship.includes_user(*id));
    if !assignees_in_relationship {
        return Err(Error::validation(
            "assignee_ids must name the coach or coachee of the action's relationship.",
        ));
    }

    let added: Vec<Id> = request
        .assignee_ids
        .iter()
        .flatten()
        .copied()
        .filter(|id| !previous.contains(id))
        .collect();
    CoachingRelationshipApi::ensure_assignees_are_members(db, relationship, &added).await?;

    let Some(goal_id) = request.action.goal_id else {
        return Ok(());
    };
    let goal = GoalApi::find_by_id(db, goal_id).await?;
    if goal.coaching_relationship_id != relationship.id {
        return Err(Error::validation(
            "goal_id must name a goal of the action's relationship.",
        ));
    }
    Ok(())
}

/// POST create a new Action
#[utoipa::path(
    post,
    path = "/actions",
    params(ApiVersion),
    request_body = ActionRequest,
    responses(
        (status = 201, description = "Successfully Created a New Action", body = domain::action::ActionWithAssignees),
        (status = 409, description = "An assignee is no longer a member of the organization; `details.reason` is `assignee_not_a_member` (with `details.user_id` and `details.organization_id`)"),
        (status= 422, description = "Unprocessable Entity"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Coaching session not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(request): Json<ActionRequest>,
) -> Result<Response, Error> {
    debug!("POST Create a New Action from: {:?}", request.action);

    let relationship =
        match authorize_session(&app_state, &user, request.action.coaching_session_id).await {
            Ok((_, relationship)) => relationship,
            Err(rejection) => return Ok(rejection.into_response()),
        };
    validate_links(app_state.db_conn_ref(), &relationship, &request, &[]).await?;

    let action = ActionApi::create_with_assignees(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        request.action,
        user.id,
        request.assignee_ids,
    )
    .await?;

    if action.has_assignees() {
        EmailsApi::notify_action_assigned(
            app_state.db_conn_ref(),
            &app_state.config,
            &action.assignee_ids,
            &user,
            &action.action,
        )
        .await;
    }

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), action)).into_response())
}

/// GET a particular Action specified by its id.
#[utoipa::path(
    get,
    path = "/actions/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Action id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a specific Action by its id", body = domain::action::ActionWithAssignees),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Action not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    ActionAccess { action, .. }: ActionAccess,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Action by id: {}", action.id);

    let assignee_ids = ActionApi::find_assignee_ids(app_state.db_conn_ref(), action.id).await?;
    let action = ActionWithAssignees {
        action,
        assignee_ids,
    };

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), action)))
}

/// Notify newly added assignees via email.
/// Compares the post-update assignee list against `previous_ids` and
/// only notifies users who were not previously assigned.
async fn notify_added_assignees(
    app_state: &AppState,
    action: &ActionWithAssignees,
    assigner: &users::Model,
    previous_ids: &[Id],
) {
    let new_assignees = action.added_assignees(previous_ids);
    if new_assignees.is_empty() {
        return;
    }

    EmailsApi::notify_action_assigned(
        app_state.db_conn_ref(),
        &app_state.config,
        &new_assignees,
        assigner,
        &action.action,
    )
    .await;
}

#[utoipa::path(
    put,
    path = "/actions/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Id of action to update"),
    ),
    request_body = ActionRequest,
    responses(
        (status = 200, description = "Successfully Updated Action", body = domain::action::ActionWithAssignees),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Action not found"),
        (status = 409, description = "An added assignee is no longer a member of the organization; `details.reason` is `assignee_not_a_member` (with `details.user_id` and `details.organization_id`)"),
        (status = 422, description = "Unprocessable Entity"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    ActionAccess {
        action,
        relationship,
    }: ActionAccess,
    State(app_state): State<AppState>,
    Json(request): Json<ActionRequest>,
) -> Result<impl IntoResponse, Error> {
    let id = action.id;
    debug!("PUT Update Action with id: {id}");

    // Current assignees, read before the update: only those added are checked and notified.
    let previous_assignee_ids = match request.have_assignees_changed() {
        true => Some(ActionApi::find_assignee_ids(app_state.db_conn_ref(), id).await?),
        false => None,
    };
    validate_links(
        app_state.db_conn_ref(),
        &relationship,
        &request,
        previous_assignee_ids.as_deref().unwrap_or_default(),
    )
    .await?;

    let action = ActionApi::update_with_assignees(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        id,
        request.action,
        request.assignee_ids,
    )
    .await?;

    debug!("Updated Action: {action:?}");

    // Best-effort email — only notify newly added assignees
    if let Some(prev_ids) = &previous_assignee_ids {
        notify_added_assignees(&app_state, &action, &user, prev_ids).await;
    }

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), action)))
}

#[utoipa::path(
    put,
    path = "/actions/{id}/status",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Id of action to update"),
        ("value" = String, Query, description = "New status: not_started, in_progress, completed, on_hold or wont_do"),
    ),
    responses(
        (status = 200, description = "Successfully Updated Action", body = domain::actions::Model),
        (status = 400, description = "Missing value"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Action not found"),
        (status = 405, description = "Method not allowed"),
        (status = 422, description = "Unknown status value"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update_status(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    ActionAccess { action, .. }: ActionAccess,
    Query(params): Query<status::UpdateParams>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let id = action.id;
    debug!("PUT Update Action Status with id: {id}");

    let action = ActionApi::update_status(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        id,
        params.status()?,
    )
    .await?;

    debug!("Updated Action: {action:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), action)))
}

#[utoipa::path(
    get,
    path = "/actions",
    params(
        ApiVersion,
        ("coaching_session_id" = Option<Uuid>, Query, description = "Filter by coaching_session_id"),
        ("goal_id" = Option<Uuid>, Query, description = "Filter by goal_id"),
        ("sort_by" = Option<crate::params::action::SortField>, Query, description = "Sort by field. Valid values: 'due_by', 'created_at', 'updated_at'. Must be provided with sort_order.", example = "due_by"),
        ("sort_order" = Option<crate::params::sort::SortOrder>, Query, description = "Sort order. Valid values: 'asc' (ascending), 'desc' (descending). Must be provided with sort_by.", example = "desc")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Actions", body = [domain::action::ActionWithAssignees]),
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
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET all Actions");
    debug!("Filter Params: {params:?}");

    // Apply default sorting parameters
    let mut params = params;
    IndexParams::apply_sort_defaults(
        &mut params.sort_by,
        &mut params.sort_order,
        SortField::DueBy,
    );

    let actions = ActionApi::find_by_with_assignees(app_state.db_conn_ref(), params).await?;

    debug!("Found Actions: {actions:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), actions)))
}

/// DELETE an Action specified by its primary key.
#[utoipa::path(
    delete,
    path = "/actions/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Action id to delete")
    ),
    responses(
        (status = 200, description = "Successfully deleted a certain Action by its id", body = serde_json::Value),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Action not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    ActionAccess { action, .. }: ActionAccess,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let id = action.id;
    debug!("DELETE Action by id: {id}");

    ActionApi::delete_by_id(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        id,
    )
    .await?;
    Ok(Json(json!({"id": id})))
}
