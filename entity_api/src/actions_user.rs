//! Entity API for actions_users junction table.
//!
//! Provides CRUD operations for managing the many-to-many relationship
//! between actions and their assigned users (coach and/or coachee).

use entity::actions_users::{ActiveModel, Column, Entity, Model};
use entity::Id;
use sea_orm::{
    entity::prelude::*, ActiveValue::Set, Condition, ConnectionTrait, DatabaseConnection,
    TransactionTrait, TryIntoModel,
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
/// Returns a vector of actions_user models containing the user IDs.
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

/// Finds all actions_user records where a specific user is assigned.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_by_user_id(db: &DatabaseConnection, user_id: Id) -> Result<Vec<Model>, Error> {
    debug!("Finding assignments for user_id={user_id}");

    let assignees = Entity::find()
        .filter(Column::UserId.eq(user_id))
        .all(db)
        .await?;

    Ok(assignees)
}

/// Returns just the action IDs assigned to a given user.
///
/// This is a convenience function that extracts only the action_id field
/// from the assignee records.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_action_ids_by_user_id(
    db: &DatabaseConnection,
    user_id: Id,
) -> Result<Vec<Id>, Error> {
    let assignees = find_by_user_id(db, user_id).await?;
    Ok(assignees.into_iter().map(|a| a.action_id).collect())
}

/// Batch fetches all assignee user IDs for multiple actions in a single query.
///
/// Returns a HashMap mapping each action_id to its list of assignee user_ids.
/// Actions with no assignees will have an empty Vec.
///
/// This is more efficient than calling `find_user_ids_by_action_id` in a loop
/// as it avoids the N+1 query problem.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_assignees_for_actions(
    db: &DatabaseConnection,
    action_ids: Vec<Id>,
) -> Result<std::collections::HashMap<Id, Vec<Id>>, Error> {
    if action_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    debug!("Batch fetching assignees for {} actions", action_ids.len());

    let assignees = Entity::find()
        .filter(Column::ActionId.is_in(action_ids))
        .all(db)
        .await?;

    // Group by action_id
    let mut map: std::collections::HashMap<Id, Vec<Id>> = std::collections::HashMap::new();
    for assignee in assignees {
        map.entry(assignee.action_id)
            .or_default()
            .push(assignee.user_id);
    }

    Ok(map)
}

/// Sets the assignees for an action, replacing any existing assignments.
///
/// This performs an atomic operation within a database transaction: deletes all
/// existing assignees and creates new records for the provided user IDs.
/// If any operation fails, the entire operation is rolled back.
///
/// # Arguments
///
/// * `db` - Database connection
/// * `action_id` - The action to update assignees for
/// * `user_ids` - The user IDs to assign (empty to remove all assignees)
///
/// # Errors
///
/// Returns `Error` if any database operation fails. On error, the transaction
/// is rolled back and no changes are persisted.
pub async fn set_assignees(
    db: &DatabaseConnection,
    action_id: Id,
    user_ids: Vec<Id>,
) -> Result<Vec<Model>, Error> {
    debug!("Setting assignees for action_id={action_id}: {user_ids:?}");

    // Use a transaction to ensure atomicity of delete + insert operations
    let txn = db.begin().await?;

    // Delete existing assignees
    delete_all_for_action(&txn, action_id).await?;

    // Create new assignees
    let now = chrono::Utc::now();
    let mut created_assignees = Vec::with_capacity(user_ids.len());

    for user_id in &user_ids {
        let active_model = ActiveModel {
            action_id: Set(action_id),
            user_id: Set(*user_id),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            ..Default::default()
        };

        let model = active_model.insert(&txn).await?.try_into_model()?;
        created_assignees.push(model);
    }

    // Commit the transaction
    txn.commit().await?;

    Ok(created_assignees)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

    #[tokio::test]
    async fn create_returns_a_new_actions_user_record() -> Result<(), Error> {
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

    /// Tests that find_by_user_id correctly retrieves all action assignments
    /// for a given user, enabling lookup of all actions a user is assigned to.
    #[tokio::test]
    async fn find_by_user_id_returns_assignments() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let user_id = Id::new_v4();

        let assignment1 = Model {
            id: Id::new_v4(),
            action_id: Id::new_v4(),
            user_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let assignment2 = Model {
            id: Id::new_v4(),
            action_id: Id::new_v4(),
            user_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![assignment1.clone(), assignment2.clone()]])
            .into_connection();

        let results = find_by_user_id(&db, user_id).await?;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].user_id, user_id);
        assert_eq!(results[1].user_id, user_id);

        Ok(())
    }

    /// Tests that find_action_ids_by_user_id extracts only the action IDs
    /// from assignments, useful for fetching full action details in a second query.
    #[tokio::test]
    async fn find_action_ids_by_user_id_returns_action_ids() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let user_id = Id::new_v4();
        let action_id_1 = Id::new_v4();
        let action_id_2 = Id::new_v4();

        let assignment1 = Model {
            id: Id::new_v4(),
            action_id: action_id_1,
            user_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let assignment2 = Model {
            id: Id::new_v4(),
            action_id: action_id_2,
            user_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![assignment1.clone(), assignment2.clone()]])
            .into_connection();

        let results = find_action_ids_by_user_id(&db, user_id).await?;

        assert_eq!(results.len(), 2);
        assert!(results.contains(&action_id_1));
        assert!(results.contains(&action_id_2));

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

    /// Tests that find_assignees_for_actions batch fetches assignees for multiple actions
    /// and correctly groups them by action_id in a HashMap.
    #[tokio::test]
    async fn find_assignees_for_actions_returns_grouped_assignees() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let action_id_1 = Id::new_v4();
        let action_id_2 = Id::new_v4();
        let user_id_1 = Id::new_v4();
        let user_id_2 = Id::new_v4();
        let user_id_3 = Id::new_v4();

        // Action 1 has 2 assignees, Action 2 has 1 assignee
        let assignee1 = Model {
            id: Id::new_v4(),
            action_id: action_id_1,
            user_id: user_id_1,
            created_at: now.into(),
            updated_at: now.into(),
        };
        let assignee2 = Model {
            id: Id::new_v4(),
            action_id: action_id_1,
            user_id: user_id_2,
            created_at: now.into(),
            updated_at: now.into(),
        };
        let assignee3 = Model {
            id: Id::new_v4(),
            action_id: action_id_2,
            user_id: user_id_3,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![
                assignee1.clone(),
                assignee2.clone(),
                assignee3.clone(),
            ]])
            .into_connection();

        let result =
            find_assignees_for_actions(&db, vec![action_id_1, action_id_2, Id::new_v4()]).await?;

        // Action 1 should have 2 assignees
        let action1_assignees = result.get(&action_id_1).unwrap();
        assert_eq!(action1_assignees.len(), 2);
        assert!(action1_assignees.contains(&user_id_1));
        assert!(action1_assignees.contains(&user_id_2));

        // Action 2 should have 1 assignee
        let action2_assignees = result.get(&action_id_2).unwrap();
        assert_eq!(action2_assignees.len(), 1);
        assert!(action2_assignees.contains(&user_id_3));

        // Action 3 (random ID with no assignees) should not be in the map
        // (map only contains actions that have assignees)
        assert!(!result.contains_key(&Id::new_v4()));

        Ok(())
    }

    /// Tests that find_assignees_for_actions returns an empty HashMap for empty input.
    #[tokio::test]
    async fn find_assignees_for_actions_returns_empty_for_empty_input() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let result = find_assignees_for_actions(&db, vec![]).await?;

        assert!(result.is_empty());

        Ok(())
    }
}
