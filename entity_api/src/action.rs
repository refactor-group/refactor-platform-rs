use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, QuerySelect, TryIntoModel,
};

use super::action_assignee;
use super::error::{EntityApiErrorKind, Error};
use entity::actions::{ActiveModel, Entity, Model};
use entity::{status::Status, Id};
use log::*;

/// An action with its associated assignee user IDs.
///
/// The frontend resolves the user names from the IDs using existing
/// coach/coachee data from the coaching relationship context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ActionWithAssignees {
    #[serde(flatten)]
    pub action: Model,
    pub assignee_ids: Vec<Id>,
}

pub async fn create(
    db: &DatabaseConnection,
    action_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    debug!("New Action Model to be inserted: {action_model:?}");

    let now = chrono::Utc::now();

    let action_active_model: ActiveModel = ActiveModel {
        coaching_session_id: Set(action_model.coaching_session_id),
        user_id: Set(user_id),
        body: Set(action_model.body),
        status: Set(action_model.status),
        due_by: Set(action_model.due_by),
        status_changed_at: Set(now.into()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(action_active_model.save(db).await?.try_into_model()?)
}

pub async fn update(db: &DatabaseConnection, id: Id, model: Model) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(action) => {
            debug!("Existing Action model to be Updated: {action:?}");

            let active_model: ActiveModel = ActiveModel {
                id: Unchanged(action.id),
                coaching_session_id: Unchanged(action.coaching_session_id),
                user_id: Unchanged(model.user_id),
                body: Set(model.body),
                due_by: Set(model.due_by),
                status: Set(model.status),
                status_changed_at: Set(chrono::Utc::now().into()),
                updated_at: Set(chrono::Utc::now().into()),
                created_at: Unchanged(action.created_at),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            error!("Action with id {id} not found");

            Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
}

pub async fn update_status(
    db: &DatabaseConnection,
    id: Id,
    status: Status,
) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(action) => {
            debug!("Existing Action model to be Updated: {action:?}");

            let active_model: ActiveModel = ActiveModel {
                id: Unchanged(action.id),
                coaching_session_id: Unchanged(action.coaching_session_id),
                user_id: Unchanged(action.user_id),
                body: Unchanged(action.body),
                due_by: Unchanged(action.due_by),
                status: Set(status),
                status_changed_at: Set(chrono::Utc::now().into()),
                updated_at: Set(chrono::Utc::now().into()),
                created_at: Unchanged(action.created_at),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            error!("Action with id {id} not found");

            Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
}

pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    let result = find_by_id(db, id).await?;

    result.delete(db).await?;

    Ok(())
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Creates a new action with optional assignees.
///
/// # Arguments
///
/// * `db` - Database connection
/// * `action_model` - The action model data
/// * `user_id` - The user ID of the action creator
/// * `assignee_ids` - Optional list of user IDs to assign to the action
///
/// # Errors
///
/// Returns `Error` if the database operation fails.
pub async fn create_with_assignees(
    db: &DatabaseConnection,
    action_model: Model,
    user_id: Id,
    assignee_ids: Option<Vec<Id>>,
) -> Result<ActionWithAssignees, Error> {
    // Create the action first
    let action = create(db, action_model, user_id).await?;

    // Set assignees if provided
    let assignee_ids = if let Some(ids) = assignee_ids {
        action_assignee::set_assignees(db, action.id, ids).await?;
        action_assignee::find_user_ids_by_action_id(db, action.id).await?
    } else {
        vec![]
    };

    Ok(ActionWithAssignees {
        action,
        assignee_ids,
    })
}

/// Updates an existing action with optional assignee changes.
///
/// # Arguments
///
/// * `db` - Database connection
/// * `id` - The action ID to update
/// * `model` - The updated action model data
/// * `assignee_ids` - Optional list of user IDs to set as assignees.
///   If `Some`, replaces existing assignees.
///   If `None`, assignees remain unchanged.
///
/// # Errors
///
/// Returns `Error` if the action is not found or database operation fails.
pub async fn update_with_assignees(
    db: &DatabaseConnection,
    id: Id,
    model: Model,
    assignee_ids: Option<Vec<Id>>,
) -> Result<ActionWithAssignees, Error> {
    // Update the action
    let action = update(db, id, model).await?;

    // Update assignees if specified
    if let Some(ids) = assignee_ids {
        action_assignee::set_assignees(db, action.id, ids).await?;
    }

    // Fetch current assignees
    let assignee_ids = action_assignee::find_user_ids_by_action_id(db, action.id).await?;

    Ok(ActionWithAssignees {
        action,
        assignee_ids,
    })
}

/// Finds an action by ID and includes its assignee IDs.
///
/// # Errors
///
/// Returns `Error` if the action is not found or database operation fails.
pub async fn find_by_id_with_assignees(
    db: &DatabaseConnection,
    id: Id,
) -> Result<ActionWithAssignees, Error> {
    let action = find_by_id(db, id).await?;
    let assignee_ids = action_assignee::find_user_ids_by_action_id(db, action.id).await?;

    Ok(ActionWithAssignees {
        action,
        assignee_ids,
    })
}

/// Filter for querying actions by assignee status.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssigneeFilter {
    /// Return all actions regardless of assignee status (default)
    #[default]
    All,
    /// Return only actions that have at least one assignee
    Assigned,
    /// Return only actions that have no assignees
    Unassigned,
}

/// Scope for user actions query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Actions assigned to this user
    Assigned,
    /// Actions from coaching sessions where user is coach or coachee (default)
    #[default]
    Sessions,
}

/// Query options for the unified user actions endpoint.
#[derive(Clone, Debug, Default)]
pub struct UserActionsQuery {
    pub scope: Scope,
    pub coaching_session_id: Option<Id>,
    pub status: Option<entity::status::Status>,
    pub assignee_filter: AssigneeFilter,
    pub sort_column: Option<entity::actions::Column>,
    pub sort_order: Option<sea_orm::Order>,
}

/// Unified query for user actions with flexible filtering and sorting.
///
/// Supports two scopes:
/// - `Scope::Assigned`: Actions where the user is an assignee
/// - `Scope::Sessions`: Actions from coaching sessions where user is coach or coachee
///
/// # Example
///
/// ```ignore
/// let actions = find_by_user(db, user_id, UserActionsQuery {
///     scope: Scope::Sessions,
///     coaching_session_id: Some(session_id),
///     status: Some(Status::InProgress),
///     assignee_filter: AssigneeFilter::All,
///     sort_column: Some(actions::Column::DueBy),
///     sort_order: Some(Order::Asc),
/// }).await?;
/// ```
pub async fn find_by_user(
    db: &DatabaseConnection,
    user_id: Id,
    query: UserActionsQuery,
) -> Result<Vec<ActionWithAssignees>, Error> {
    use entity::{actions, coaching_relationships, coaching_sessions};
    use sea_orm::{JoinType, QueryOrder};

    debug!(
        "Finding actions for user_id={user_id} with scope={:?}, session={:?}, status={:?}, assignee={:?}",
        query.scope, query.coaching_session_id, query.status, query.assignee_filter
    );

    // Build base query based on scope
    let base_select = match query.scope {
        Scope::Assigned => {
            let action_ids = action_assignee::find_action_ids_by_user_id(db, user_id).await?;

            if action_ids.is_empty() {
                return Ok(vec![]);
            }

            actions::Entity::find().filter(actions::Column::Id.is_in(action_ids))
        }
        Scope::Sessions => actions::Entity::find()
            .join(
                JoinType::InnerJoin,
                actions::Relation::CoachingSessions.def(),
            )
            .join(
                JoinType::InnerJoin,
                coaching_sessions::Relation::CoachingRelationships.def(),
            )
            .filter(
                coaching_relationships::Column::CoachId
                    .eq(user_id)
                    .or(coaching_relationships::Column::CoacheeId.eq(user_id)),
            ),
    };

    // Apply filters
    let mut select = base_select;

    if let Some(session_id) = query.coaching_session_id {
        select = select.filter(actions::Column::CoachingSessionId.eq(session_id));
    }

    if let Some(status) = &query.status {
        select = select.filter(actions::Column::Status.eq(status.clone()));
    }

    // Apply sorting
    if let (Some(column), Some(order)) = (query.sort_column, query.sort_order) {
        select = select.order_by(column, order);
    }

    let actions: Vec<entity::actions::Model> = select.all(db).await?;

    // Build ActionWithAssignees and apply assignee filter
    let mut results = Vec::with_capacity(actions.len());
    for action in actions {
        let assignee_ids = action_assignee::find_user_ids_by_action_id(db, action.id).await?;

        let include = match query.assignee_filter {
            AssigneeFilter::All => true,
            AssigneeFilter::Assigned => !assignee_ids.is_empty(),
            AssigneeFilter::Unassigned => assignee_ids.is_empty(),
        };

        if include {
            results.push(ActionWithAssignees {
                action,
                assignee_ids,
            });
        }
    }

    debug!(
        "Found {} actions for user {user_id} (scope={:?})",
        results.len(),
        query.scope
    );

    Ok(results)
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::{actions::Model, Id};
    use sea_orm::{DatabaseBackend, MockDatabase};

    #[tokio::test]
    async fn create_returns_a_new_action_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let action_model = Model {
            id: Id::new_v4(),
            user_id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            body: Some("This is a action".to_owned()),
            due_by: Some(now.into()),
            status_changed_at: now.into(),
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![action_model.clone()]])
            .into_connection();

        let action = create(&db, action_model.clone().into(), Id::new_v4()).await?;

        assert_eq!(action.id, action_model.id);

        Ok(())
    }

    #[tokio::test]
    async fn update_returns_an_updated_action_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let action_model = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            due_by: Some(now.into()),
            body: Some("This is a action".to_owned()),
            user_id: Id::new_v4(),
            status_changed_at: now.into(),
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![action_model.clone()], vec![action_model.clone()]])
            .into_connection();

        let action = update(&db, action_model.id, action_model.clone()).await?;

        assert_eq!(action.body, action_model.body);

        Ok(())
    }

    #[tokio::test]
    async fn update_status_returns_an_updated_action_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let action_model = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            due_by: Some(now.into()),
            body: Some("This is a action".to_owned()),
            user_id: Id::new_v4(),
            status_changed_at: now.into(),
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let updated_action_model = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            due_by: Some(now.into()),
            body: Some("This is a action".to_owned()),
            user_id: Id::new_v4(),
            status_changed_at: now.into(),
            status: Status::Completed,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![action_model.clone()],
                vec![updated_action_model.clone()],
            ])
            .into_connection();

        let action = update_status(&db, action_model.id, Status::Completed).await?;

        assert_eq!(action.status, Status::Completed);

        Ok(())
    }

    #[tokio::test]
    async fn update_status_returns_error_when_action_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let result = update_status(&db, Id::new_v4(), Status::Completed).await;

        assert_eq!(result.is_err(), true);

        Ok(())
    }

    /// Tests that find_by_user with Scope::Assigned returns actions assigned to the user.
    #[tokio::test]
    async fn find_by_user_with_scope_assigned_returns_assigned_actions() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let user_id = Id::new_v4();
        let action_id = Id::new_v4();

        let action_model = Model {
            id: action_id,
            user_id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            body: Some("Assigned action".to_owned()),
            due_by: Some(now.into()),
            status_changed_at: now.into(),
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        // Mock: 1) action_assignee query returns action IDs, 2) actions query, 3) assignee lookup
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![entity::action_assignees::Model {
                id: Id::new_v4(),
                action_id,
                user_id,
                created_at: now.into(),
                updated_at: now.into(),
            }]])
            .append_query_results(vec![vec![action_model.clone()]])
            .append_query_results(vec![vec![entity::action_assignees::Model {
                id: Id::new_v4(),
                action_id,
                user_id,
                created_at: now.into(),
                updated_at: now.into(),
            }]])
            .into_connection();

        let query = UserActionsQuery {
            scope: Scope::Assigned,
            assignee_filter: AssigneeFilter::All,
            ..Default::default()
        };

        let actions = find_by_user(&db, user_id, query).await?;

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action.id, action_id);
        assert_eq!(actions[0].assignee_ids, vec![user_id]);

        Ok(())
    }

    /// Tests that find_by_user with Scope::Assigned returns empty when user has no assignments.
    #[tokio::test]
    async fn find_by_user_with_scope_assigned_returns_empty_when_no_assignments(
    ) -> Result<(), Error> {
        let user_id = Id::new_v4();

        // Mock: action_assignee query returns empty
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<entity::action_assignees::Model>::new()])
            .into_connection();

        let query = UserActionsQuery {
            scope: Scope::Assigned,
            ..Default::default()
        };

        let actions = find_by_user(&db, user_id, query).await?;

        assert!(actions.is_empty());

        Ok(())
    }

    /// Tests that find_by_user with Scope::Sessions returns actions from user's coaching sessions.
    #[tokio::test]
    async fn find_by_user_with_scope_sessions_returns_session_actions() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let user_id = Id::new_v4();
        let action_id = Id::new_v4();

        let action_model = Model {
            id: action_id,
            user_id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            body: Some("Session action".to_owned()),
            due_by: Some(now.into()),
            status_changed_at: now.into(),
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        // Mock: 1) actions join query, 2) assignee lookup for each action
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![action_model.clone()]])
            .append_query_results(vec![vec![entity::action_assignees::Model {
                id: Id::new_v4(),
                action_id,
                user_id,
                created_at: now.into(),
                updated_at: now.into(),
            }]])
            .into_connection();

        let query = UserActionsQuery {
            scope: Scope::Sessions,
            assignee_filter: AssigneeFilter::All,
            ..Default::default()
        };

        let actions = find_by_user(&db, user_id, query).await?;

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action.body, Some("Session action".to_owned()));

        Ok(())
    }

    /// Tests that AssigneeFilter::Unassigned filters out actions with assignees.
    #[tokio::test]
    async fn find_by_user_with_assignee_filter_unassigned_excludes_assigned() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let user_id = Id::new_v4();
        let action_id = Id::new_v4();

        let action_model = Model {
            id: action_id,
            user_id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            body: Some("Action with assignee".to_owned()),
            due_by: Some(now.into()),
            status_changed_at: now.into(),
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        // Mock: action has an assignee, so should be filtered out
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![action_model.clone()]])
            .append_query_results(vec![vec![entity::action_assignees::Model {
                id: Id::new_v4(),
                action_id,
                user_id: Id::new_v4(), // Has an assignee
                created_at: now.into(),
                updated_at: now.into(),
            }]])
            .into_connection();

        let query = UserActionsQuery {
            scope: Scope::Sessions,
            assignee_filter: AssigneeFilter::Unassigned,
            ..Default::default()
        };

        let actions = find_by_user(&db, user_id, query).await?;

        // Should be empty because the action has an assignee
        assert!(actions.is_empty());

        Ok(())
    }

    /// Tests that AssigneeFilter::Assigned filters out actions without assignees.
    #[tokio::test]
    async fn find_by_user_with_assignee_filter_assigned_excludes_unassigned() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let user_id = Id::new_v4();
        let action_id = Id::new_v4();

        let action_model = Model {
            id: action_id,
            user_id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            body: Some("Action without assignee".to_owned()),
            due_by: Some(now.into()),
            status_changed_at: now.into(),
            status: Default::default(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        // Mock: action has no assignees, so should be filtered out
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![action_model.clone()]])
            .append_query_results(vec![Vec::<entity::action_assignees::Model>::new()]) // No assignees
            .into_connection();

        let query = UserActionsQuery {
            scope: Scope::Sessions,
            assignee_filter: AssigneeFilter::Assigned,
            ..Default::default()
        };

        let actions = find_by_user(&db, user_id, query).await?;

        // Should be empty because the action has no assignees
        assert!(actions.is_empty());

        Ok(())
    }
}
