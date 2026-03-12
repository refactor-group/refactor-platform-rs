use super::error::{EntityApiErrorKind, Error};
use entity::goals::{ActiveModel, Column, Entity, Model};
use entity::{status::Status, Id};
use sea_orm::ActiveValue;
use sea_orm::{
    entity::prelude::*,
    ActiveModelTrait,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, QueryFilter, TransactionTrait, TryIntoModel,
};

use log::*;

/// Maximum number of active (`InProgress`) goals allowed per coaching relationship.
const MAX_ACTIVE_GOALS: usize = 3;

/// Returns the maximum number of active (`InProgress`) goals allowed per coaching relationship.
pub fn max_active_goals() -> usize {
    MAX_ACTIVE_GOALS
}

pub async fn create(
    db: &DatabaseConnection,
    goal_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    debug!("New Goal Model to be inserted: {goal_model:?}");

    if goal_model.in_progress() {
        check_active_goal_limit(db, goal_model.coaching_relationship_id).await?;
    }

    let now = chrono::Utc::now();

    let goal_active_model: ActiveModel = ActiveModel {
        coaching_relationship_id: Set(goal_model.coaching_relationship_id),
        created_in_session_id: Set(goal_model.created_in_session_id),
        user_id: Set(user_id),
        title: Set(goal_model.title),
        body: Set(goal_model.body),
        status: Set(goal_model.status),
        status_changed_at: Set(Some(now.into())),
        completed_at: Unchanged(goal_model.completed_at),
        target_date: Set(goal_model.target_date),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(goal_active_model.save(db).await?.try_into_model()?)
}

pub async fn update(db: &DatabaseConnection, id: Id, model: Model) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(goal) => {
            debug!("Existing Goal model to be Updated: {goal:?}");

            // Check active goal limit if transitioning into active from a non-active status.
            if model.in_progress() && !goal.in_progress() {
                check_active_goal_limit(db, goal.coaching_relationship_id).await?;
            }

            // Automatically update status_changed_at if the last status and new status differ:
            let av_status_changed_at: ActiveValue<Option<DateTimeWithTimeZone>> =
                if model.status != goal.status {
                    debug!("Updating status_changed_at for Goal to now");
                    Set(Some(chrono::Utc::now().into()))
                } else {
                    Unchanged(model.status_changed_at)
                };

            let active_model: ActiveModel = ActiveModel {
                id: Unchanged(goal.id),
                coaching_relationship_id: Unchanged(goal.coaching_relationship_id),
                created_in_session_id: Unchanged(goal.created_in_session_id),
                user_id: Unchanged(goal.user_id),
                body: Set(model.body),
                title: Set(model.title),
                status: Set(model.status),
                status_changed_at: av_status_changed_at,
                completed_at: Set(model.completed_at),
                target_date: Set(model.target_date),
                updated_at: Set(chrono::Utc::now().into()),
                created_at: Unchanged(goal.created_at),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            error!("Goal with id {id} not found");

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
        Some(goal) => {
            debug!("Existing Goal model to be Updated: {goal:?}");

            // Check active goal limit if transitioning into active from a non-active status.
            if status == Status::InProgress && !goal.in_progress() {
                check_active_goal_limit(db, goal.coaching_relationship_id).await?;
            }

            let active_model: ActiveModel = ActiveModel {
                id: Unchanged(goal.id),
                coaching_relationship_id: Unchanged(goal.coaching_relationship_id),
                created_in_session_id: Unchanged(goal.created_in_session_id),
                user_id: Unchanged(goal.user_id),
                body: Unchanged(goal.body),
                title: Unchanged(goal.title),
                status: Set(status),
                status_changed_at: Set(Some(chrono::Utc::now().into())),
                completed_at: Unchanged(goal.completed_at),
                target_date: Unchanged(goal.target_date),
                updated_at: Set(chrono::Utc::now().into()),
                created_at: Unchanged(goal.created_at),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            error!("Goal with id {id} not found");

            Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
}

pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    let txn = db.begin().await?;
    let goal = Entity::find_by_id(id)
        .one(&txn)
        .await?
        .ok_or_else(|| Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        })?;
    Entity::delete_by_id(id).exec(&txn).await?;
    txn.commit().await?;
    Ok(goal)
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Finds all active goals (`InProgress` status) for a given coaching relationship.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_active_goals_by_coaching_relationship_id(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::CoachingRelationshipId.eq(coaching_relationship_id))
        .filter(Column::Status.eq(Status::InProgress))
        .all(db)
        .await?)
}

/// Checks that adding one more `InProgress` goal to a coaching relationship
/// would not exceed `MAX_ACTIVE_GOALS`. Returns a `ValidationError` carrying
/// summaries of the current active goals so the caller can present a "swap" dialog.
async fn check_active_goal_limit(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
) -> Result<(), Error> {
    let active_goals =
        find_active_goals_by_coaching_relationship_id(db, coaching_relationship_id).await?;

    if active_goals.len() >= MAX_ACTIVE_GOALS {
        let summaries: Vec<serde_json::Value> = active_goals
            .iter()
            .map(|g| {
                serde_json::json!({
                    "id": g.id,
                    "title": g.title.clone().unwrap_or_default(),
                })
            })
            .collect();

        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::ValidationError {
                message: format!(
                    "A coaching relationship can have at most {MAX_ACTIVE_GOALS} active goals."
                ),
                details: Some(serde_json::json!({
                    "max_active_goals": MAX_ACTIVE_GOALS,
                    "active_goals": summaries,
                })),
            },
        });
    }

    Ok(())
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::{goals::Model, Id};
    use sea_orm::{DatabaseBackend, MockDatabase};

    #[tokio::test]
    async fn create_returns_a_new_goal_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let goal_model = Model {
            id: Id::new_v4(),
            user_id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            created_in_session_id: Some(Id::new_v4()),
            title: Some("title".to_owned()),
            body: Some("This is a goal".to_owned()),
            status_changed_at: None,
            status: Default::default(),
            completed_at: Some(now.into()),
            target_date: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        // Default status is InProgress, so the limit check runs first (returns empty → under limit)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<Model>::new()])
            .append_query_results(vec![vec![goal_model.clone()]])
            .into_connection();

        let goal = create(&db, goal_model.clone().into(), Id::new_v4()).await?;

        assert_eq!(goal.id, goal_model.id);

        Ok(())
    }

    #[tokio::test]
    async fn update_returns_an_updated_goal_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let goal_model = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            created_in_session_id: Some(Id::new_v4()),
            title: Some("title".to_owned()),
            body: Some("This is a goal".to_owned()),
            user_id: Id::new_v4(),
            completed_at: Some(now.into()),
            status_changed_at: None,
            status: Default::default(),
            target_date: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![goal_model.clone()], vec![goal_model.clone()]])
            .into_connection();

        let goal = update(&db, goal_model.id, goal_model.clone()).await?;

        assert_eq!(goal.body, goal_model.body);

        Ok(())
    }

    #[tokio::test]
    async fn update_status_returns_an_updated_goal_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let goal_model = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            created_in_session_id: Some(Id::new_v4()),
            title: Some("title".to_owned()),
            body: Some("This is a goal".to_owned()),
            user_id: Id::new_v4(),
            completed_at: Some(now.into()),
            status_changed_at: None,
            status: Default::default(),
            target_date: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let updated_goal_model = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            created_in_session_id: Some(Id::new_v4()),
            title: Some("title".to_owned()),
            body: Some("This is a goal".to_owned()),
            user_id: Id::new_v4(),
            completed_at: Some(now.into()),
            status_changed_at: Some(now.into()),
            status: Status::Completed,
            target_date: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![goal_model.clone()],
                vec![updated_goal_model.clone()],
            ])
            .into_connection();

        let goal = update_status(&db, goal_model.id, Status::Completed).await?;

        assert_eq!(goal.status, Status::Completed);

        Ok(())
    }

    #[tokio::test]
    async fn update_status_returns_error_when_goal_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let result = update_status(&db, Id::new_v4(), Status::Completed).await;

        assert_eq!(result.is_err(), true);

        Ok(())
    }

    #[tokio::test]
    async fn find_active_goals_returns_only_in_progress() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let relationship_id = Id::new_v4();

        let in_progress_goal = Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            created_in_session_id: None,
            user_id: Id::new_v4(),
            title: Some("Active goal".to_owned()),
            body: None,
            status: Status::InProgress,
            status_changed_at: None,
            completed_at: None,
            target_date: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![in_progress_goal.clone()]])
            .into_connection();

        let results = find_active_goals_by_coaching_relationship_id(&db, relationship_id).await?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, Status::InProgress);

        Ok(())
    }

    #[tokio::test]
    async fn find_active_goals_returns_empty_when_none() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<Model>::new()])
            .into_connection();

        let results = find_active_goals_by_coaching_relationship_id(&db, Id::new_v4()).await?;

        assert!(results.is_empty());

        Ok(())
    }

    fn create_test_goal(
        status: Status,
        title: Option<String>,
        coaching_relationship_id: Id,
    ) -> Model {
        let now = chrono::Utc::now();
        Model {
            id: Id::new_v4(),
            coaching_relationship_id,
            created_in_session_id: None,
            user_id: Id::new_v4(),
            title,
            body: None,
            status,
            status_changed_at: None,
            completed_at: None,
            target_date: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    #[tokio::test]
    async fn create_rejects_in_progress_when_at_limit() {
        let relationship_id = Id::new_v4();

        let active_goals: Vec<Model> = (0..MAX_ACTIVE_GOALS)
            .map(|i| {
                create_test_goal(
                    Status::InProgress,
                    Some(format!("Goal {i}")),
                    relationship_id,
                )
            })
            .collect();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![active_goals])
            .into_connection();

        let new_goal = create_test_goal(
            Status::InProgress,
            Some("One too many".to_string()),
            relationship_id,
        );

        let result = create(&db, new_goal, Id::new_v4()).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::ValidationError { .. }
        ));
    }

    #[tokio::test]
    async fn create_allows_in_progress_when_under_limit() {
        let relationship_id = Id::new_v4();

        let active_goals: Vec<Model> = (0..MAX_ACTIVE_GOALS - 1)
            .map(|i| {
                create_test_goal(
                    Status::InProgress,
                    Some(format!("Goal {i}")),
                    relationship_id,
                )
            })
            .collect();

        let new_goal = create_test_goal(
            Status::InProgress,
            Some("Fits under limit".to_string()),
            relationship_id,
        );

        // Mock sequence: active goals query → goal save
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![active_goals])
            .append_query_results(vec![vec![new_goal.clone()]])
            .into_connection();

        let result = create(&db, new_goal, Id::new_v4()).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn create_allows_not_started_even_at_limit() {
        let relationship_id = Id::new_v4();

        // NotStarted bypasses the limit check — no active goals query needed
        let new_goal = create_test_goal(
            Status::NotStarted,
            Some("Queued goal".to_string()),
            relationship_id,
        );

        // Mock sequence: goal save only (no limit check)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![new_goal.clone()]])
            .into_connection();

        let result = create(&db, new_goal, Id::new_v4()).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn update_status_rejects_in_progress_when_at_limit() {
        let relationship_id = Id::new_v4();

        let current_goal = create_test_goal(
            Status::NotStarted,
            Some("My goal".to_string()),
            relationship_id,
        );

        let active_goals: Vec<Model> = (0..MAX_ACTIVE_GOALS)
            .map(|i| {
                create_test_goal(
                    Status::InProgress,
                    Some(format!("Active {i}")),
                    relationship_id,
                )
            })
            .collect();

        // Mock sequence: find_by_id (current goal) → active goals query → error
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![current_goal.clone()]])
            .append_query_results(vec![active_goals])
            .into_connection();

        let result = update_status(&db, current_goal.id, Status::InProgress).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::ValidationError { .. }
        ));
    }

    #[tokio::test]
    async fn update_status_allows_in_progress_to_in_progress() {
        let relationship_id = Id::new_v4();

        // Goal is already InProgress — no-op transition, skips limit check
        let current_goal = create_test_goal(
            Status::InProgress,
            Some("Already active".to_string()),
            relationship_id,
        );

        // Mock sequence: find_by_id → already InProgress so skip limit check → save
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![current_goal.clone()]])
            .append_query_results(vec![vec![current_goal.clone()]])
            .into_connection();

        let result = update_status(&db, current_goal.id, Status::InProgress).await;

        assert!(result.is_ok());
    }
}
