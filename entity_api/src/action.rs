use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, TryIntoModel,
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

/// Finds all actions assigned to a specific user across all coaching sessions.
///
/// This queries the `action_assignees` junction table to find all actions
/// where the given user is an assignee, then fetches the full action data
/// with all assignee IDs for each action.
///
/// # Arguments
///
/// * `db` - Database connection
/// * `user_id` - The user ID to find assigned actions for
///
/// # Returns
///
/// A vector of `ActionWithAssignees` containing each action and its assignee IDs.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_by_assignee_with_assignees(
    db: &DatabaseConnection,
    user_id: Id,
) -> Result<Vec<ActionWithAssignees>, Error> {
    debug!("Finding actions assigned to user_id={user_id}");

    // Get all action IDs where this user is assigned
    let action_ids = action_assignee::find_action_ids_by_user_id(db, user_id).await?;

    if action_ids.is_empty() {
        return Ok(vec![]);
    }

    // Fetch all actions by their IDs
    let actions = Entity::find()
        .filter(entity::actions::Column::Id.is_in(action_ids))
        .all(db)
        .await?;

    // Build ActionWithAssignees for each action
    let mut results = Vec::with_capacity(actions.len());
    for action in actions {
        let assignee_ids = action_assignee::find_user_ids_by_action_id(db, action.id).await?;
        results.push(ActionWithAssignees {
            action,
            assignee_ids,
        });
    }

    debug!("Found {} actions assigned to user {user_id}", results.len());

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
}
