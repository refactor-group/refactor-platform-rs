//! Entity API for action_assignees junction table.
//!
//! Provides CRUD operations for managing the many-to-many relationship
//! between actions and their assigned users (coach and/or coachee).

use entity::action_assignees::{ActiveModel, Column, Entity, Model};
use entity::Id;
use sea_orm::{
    entity::prelude::*, ActiveValue::Set, Condition, ConnectionTrait, DatabaseConnection,
    TryIntoModel,
};

use log::*;

use super::error::Error;

/// Creates a new action assignee record.
///
/// # Errors
///
/// Returns `Error` if the database insert fails (e.g., duplicate assignment
/// or foreign key constraint violation).
pub async fn create(db: &DatabaseConnection, action_id: Id, user_id: Id) -> Result<Model, Error> {
    debug!("Creating action assignee: action_id={action_id}, user_id={user_id}");

    let now = chrono::Utc::now();

    let active_model = ActiveModel {
        action_id: Set(action_id),
        user_id: Set(user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.insert(db).await?.try_into_model()?)
}

/// Deletes a specific action assignee by action_id and user_id.
///
/// # Errors
///
/// Returns `Error` if the database delete fails.
pub async fn delete(db: &impl ConnectionTrait, action_id: Id, user_id: Id) -> Result<(), Error> {
    debug!("Deleting action assignee: action_id={action_id}, user_id={user_id}");

    Entity::delete_many()
        .filter(
            Condition::all()
                .add(Column::ActionId.eq(action_id))
                .add(Column::UserId.eq(user_id)),
        )
        .exec(db)
        .await?;

    Ok(())
}

/// Deletes all assignees for a given action.
///
/// # Errors
///
/// Returns `Error` if the database delete fails.
pub async fn delete_all_for_action(db: &impl ConnectionTrait, action_id: Id) -> Result<(), Error> {
    debug!("Deleting all assignees for action_id={action_id}");

    Entity::delete_many()
        .filter(Column::ActionId.eq(action_id))
        .exec(db)
        .await?;

    Ok(())
}

/// Finds all assignees for a given action.
///
/// Returns a vector of action_assignee models containing the user IDs.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_by_action_id(
    db: &DatabaseConnection,
    action_id: Id,
) -> Result<Vec<Model>, Error> {
    debug!("Finding assignees for action_id={action_id}");

    let assignees = Entity::find()
        .filter(Column::ActionId.eq(action_id))
        .all(db)
        .await?;

    Ok(assignees)
}

/// Returns just the user IDs assigned to a given action.
///
/// This is a convenience function that extracts only the user_id field
/// from the assignee records.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_user_ids_by_action_id(
    db: &DatabaseConnection,
    action_id: Id,
) -> Result<Vec<Id>, Error> {
    let assignees = find_by_action_id(db, action_id).await?;
    Ok(assignees.into_iter().map(|a| a.user_id).collect())
}

/// Sets the assignees for an action, replacing any existing assignments.
///
/// This performs an atomic operation: deletes all existing assignees and
/// creates new records for the provided user IDs.
///
/// # Arguments
///
/// * `db` - Database connection (should be a transaction for atomicity)
/// * `action_id` - The action to update assignees for
/// * `user_ids` - The user IDs to assign (empty to remove all assignees)
///
/// # Errors
///
/// Returns `Error` if any database operation fails.
pub async fn set_assignees(
    db: &DatabaseConnection,
    action_id: Id,
    user_ids: Vec<Id>,
) -> Result<Vec<Model>, Error> {
    debug!(
        "Setting assignees for action_id={action_id}: {:?}",
        user_ids
    );

    // Delete existing assignees
    delete_all_for_action(db, action_id).await?;

    // Create new assignees
    let now = chrono::Utc::now();
    let mut created_assignees = Vec::with_capacity(user_ids.len());

    for user_id in user_ids {
        let active_model = ActiveModel {
            action_id: Set(action_id),
            user_id: Set(user_id),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            ..Default::default()
        };

        let model = active_model.insert(db).await?.try_into_model()?;
        created_assignees.push(model);
    }

    Ok(created_assignees)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

    #[tokio::test]
    async fn create_returns_a_new_action_assignee() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let action_id = Id::new_v4();
        let user_id = Id::new_v4();

        let expected_model = Model {
            id: Id::new_v4(),
            action_id,
            user_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expected_model.clone()]])
            .into_connection();

        let result = create(&db, action_id, user_id).await?;

        assert_eq!(result.action_id, action_id);
        assert_eq!(result.user_id, user_id);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_action_id_returns_assignees() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let action_id = Id::new_v4();

        let assignee1 = Model {
            id: Id::new_v4(),
            action_id,
            user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let assignee2 = Model {
            id: Id::new_v4(),
            action_id,
            user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![assignee1.clone(), assignee2.clone()]])
            .into_connection();

        let results = find_by_action_id(&db, action_id).await?;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].action_id, action_id);
        assert_eq!(results[1].action_id, action_id);

        Ok(())
    }

    #[tokio::test]
    async fn delete_all_for_action_executes_successfully() -> Result<(), Error> {
        let action_id = Id::new_v4();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 2,
            }])
            .into_connection();

        delete_all_for_action(&db, action_id).await?;

        Ok(())
    }
}
