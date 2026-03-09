//! Entity API for coaching_sessions_goals junction table.
//!
//! Provides CRUD operations for managing the many-to-many relationship
//! between coaching sessions and goals.

use entity::coaching_sessions_goals::{Column, Entity, Model};
use entity::Id;
use sea_orm::{entity::prelude::*, ActiveValue::Set, DatabaseConnection, ModelTrait, TryIntoModel};

use log::*;

use super::error::{EntityApiErrorKind, Error};

/// Links a goal to a coaching session.
///
/// # Errors
///
/// Returns `Error` if the database insert fails (e.g., duplicate link
/// or foreign key constraint violation).
pub async fn create(
    db: &DatabaseConnection,
    coaching_session_id: Id,
    goal_id: Id,
) -> Result<Model, Error> {
    debug!("Linking goal {goal_id} to session {coaching_session_id}");

    let now = chrono::Utc::now();

    let active_model = entity::coaching_sessions_goals::ActiveModel {
        coaching_session_id: Set(coaching_session_id),
        goal_id: Set(goal_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.insert(db).await?.try_into_model()?)
}

/// Unlinks a goal from a coaching session by the join table record id.
///
/// # Errors
///
/// Returns `Error` if the record is not found or the database delete fails.
pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    let record = find_by_id(db, id).await?;
    record.delete(db).await?;
    Ok(())
}

/// Finds a specific coaching_sessions_goals record by id.
///
/// # Errors
///
/// Returns `Error` with `RecordNotFound` if no record exists with the given id.
pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Finds all goals linked to a given coaching session.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_by_session_id(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Result<Vec<Model>, Error> {
    debug!("Finding goals linked to session {coaching_session_id}");

    Ok(Entity::find()
        .filter(Column::CoachingSessionId.eq(coaching_session_id))
        .all(db)
        .await?)
}

/// Finds all sessions linked to a given goal.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_by_goal_id(db: &DatabaseConnection, goal_id: Id) -> Result<Vec<Model>, Error> {
    debug!("Finding sessions linked to goal {goal_id}");

    Ok(Entity::find()
        .filter(Column::GoalId.eq(goal_id))
        .all(db)
        .await?)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase};

    #[tokio::test]
    async fn create_returns_a_new_link_record() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let session_id = Id::new_v4();
        let goal_id = Id::new_v4();

        let expected_model = Model {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            goal_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expected_model.clone()]])
            .into_connection();

        let result = create(&db, session_id, goal_id).await?;

        assert_eq!(result.coaching_session_id, session_id);
        assert_eq!(result.goal_id, goal_id);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_session_id_returns_linked_goals() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let session_id = Id::new_v4();

        let link1 = Model {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            goal_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let link2 = Model {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            goal_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![link1.clone(), link2.clone()]])
            .into_connection();

        let results = find_by_session_id(&db, session_id).await?;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].coaching_session_id, session_id);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_goal_id_returns_linked_sessions() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let goal_id = Id::new_v4();

        let link = Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            goal_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![link.clone()]])
            .into_connection();

        let results = find_by_goal_id(&db, goal_id).await?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].goal_id, goal_id);

        Ok(())
    }
}
