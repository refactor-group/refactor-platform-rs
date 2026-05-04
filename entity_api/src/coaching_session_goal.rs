//! Entity API for coaching_sessions_goals junction table.
//!
//! Provides CRUD operations for managing the many-to-many relationship
//! between coaching sessions and goals.

use std::collections::HashMap;

use entity::coaching_sessions_goals::{Column, Entity, Model};
use entity::links::SessionGoalToCoachingRelationship;
use entity::status::Status;
use entity::{coaching_relationships, coaching_sessions, goals, Id};
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    ConnectionTrait, DatabaseConnection, TryIntoModel,
};

use log::*;

use super::error::{EntityApiErrorKind, Error};

/// Links a goal to a coaching session, enforcing the invariant that a session-linked
/// goal must be `InProgress`.
///
/// Behavior:
/// - `NotStarted` / `OnHold` goal → auto-promoted to `InProgress` (status_changed_at bumped)
///   atomically with the link insert. Returns the promoted goal alongside the link so
///   callers can publish a `GoalUpdated` event.
/// - `InProgress` goal → just inserts the link, no status change.
/// - `Completed` / `WontDo` goal → returns `CannotLinkCompletedGoal`, no writes.
/// - Auto-promotion that would push the relationship past `MAX_IN_PROGRESS_GOALS`
///   returns the standard cap-collision `ValidationError`, no writes.
///
/// Callers should pass a transaction handle so the link insert and any status
/// promotion succeed-or-fail atomically.
///
/// # Errors
///
/// Returns `Error` if the goal is not found, is in a completed status, would
/// exceed the in-progress goal cap on auto-promotion, or any database
/// query/insert fails (e.g., duplicate link or foreign key violation).
pub async fn create(
    db: &impl ConnectionTrait,
    coaching_session_id: Id,
    goal_id: Id,
) -> Result<(Model, Option<goals::Model>), Error> {
    debug!("Linking goal {goal_id} to session {coaching_session_id}");

    let goal = goals::Entity::find_by_id(goal_id)
        .one(db)
        .await?
        .ok_or(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        })?;

    if goal.is_completed() {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::CannotLinkCompletedGoal,
        });
    }

    // Reject duplicate links explicitly so the FE sees a structured 409 instead
    // of the unique-constraint violation falling through to a 503.
    // (Race window: two concurrent requests can both pass this check; the DB's
    // unique constraint on (coaching_session_id, goal_id) still protects integrity,
    // and the loser falls through to the existing 503 path. Acceptable for now.)
    let already_linked = Entity::find()
        .filter(Column::CoachingSessionId.eq(coaching_session_id))
        .filter(Column::GoalId.eq(goal_id))
        .one(db)
        .await?
        .is_some();
    if already_linked {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::GoalAlreadyLinkedToSession,
        });
    }

    let needs_promotion = !goal.in_progress();

    if needs_promotion {
        super::goal::check_in_progress_goal_limit(db, goal.coaching_relationship_id).await?;
    }

    let now = chrono::Utc::now();
    let link = insert_link_row(db, coaching_session_id, goal_id, now).await?;

    let promoted_goal = if needs_promotion {
        let promoted = goals::ActiveModel {
            id: Unchanged(goal.id),
            coaching_relationship_id: Unchanged(goal.coaching_relationship_id),
            created_in_session_id: Unchanged(goal.created_in_session_id),
            user_id: Unchanged(goal.user_id),
            title: Unchanged(goal.title.clone()),
            body: Unchanged(goal.body.clone()),
            status: Set(Status::InProgress),
            status_changed_at: Set(Some(now.into())),
            completed_at: Unchanged(goal.completed_at),
            target_date: Unchanged(goal.target_date),
            created_at: Unchanged(goal.created_at),
            updated_at: Set(now.into()),
        };
        Some(promoted.update(db).await?.try_into_model()?)
    } else {
        None
    };

    Ok((link, promoted_goal))
}

/// Inserts a row into `coaching_sessions_goals` without enforcing the
/// session-link-implies-`InProgress` invariant. Reserved for callers that have
/// already established the goal is `InProgress` (e.g. the auto-link path
/// from session-create, which queries goals filtered by `Status::InProgress`).
/// Public callers must use [`create`] instead.
async fn insert_link_row(
    db: &impl ConnectionTrait,
    coaching_session_id: Id,
    goal_id: Id,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Model, Error> {
    Ok(entity::coaching_sessions_goals::ActiveModel {
        coaching_session_id: Set(coaching_session_id),
        goal_id: Set(goal_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .insert(db)
    .await?
    .try_into_model()?)
}

/// Finds a single linked goal to a coaching session join-table record by its primary key.
///
/// # Errors
///
/// Returns `Error` with `RecordNotFound` if the record does not exist.
pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Finds a join-table record by id and eagerly loads the coaching relationship
/// it belongs to (via coaching_sessions_goals → coaching_sessions → coaching_relationships).
///
/// This uses a single query with two JOINs, avoiding separate lookups.
///
/// # Errors
///
/// Returns `Error` with `RecordNotFound` if the record or its relationship does not exist.
pub async fn find_by_id_with_coaching_relationship(
    db: &DatabaseConnection,
    id: Id,
) -> Result<(Model, coaching_relationships::Model), Error> {
    let (link, relationship) = Entity::find_by_id(id)
        .find_also_linked(SessionGoalToCoachingRelationship)
        .one(db)
        .await?
        .ok_or(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        })?;

    let relationship = relationship.ok_or(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })?;

    Ok((link, relationship))
}

/// Unlinks a goal from a coaching session by the join table record id.
///
/// # Errors
///
/// Returns `Error` if the record is not found or the database delete fails.
pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    let result = Entity::delete_by_id(id).exec(db).await?;

    if result.rows_affected == 0 {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        });
    }

    Ok(())
}

/// Finds a join-table record by the (coaching_session_id, goal_id) pair
/// and eagerly loads the coaching relationship.
///
/// # Errors
///
/// Returns `Error` with `RecordNotFound` if no link exists for the pair.
pub async fn find_by_session_and_goal_with_coaching_relationship(
    db: &DatabaseConnection,
    coaching_session_id: Id,
    goal_id: Id,
) -> Result<(Model, coaching_relationships::Model), Error> {
    let (link, relationship) = Entity::find()
        .filter(Column::CoachingSessionId.eq(coaching_session_id))
        .filter(Column::GoalId.eq(goal_id))
        .find_also_linked(SessionGoalToCoachingRelationship)
        .one(db)
        .await?
        .ok_or(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        })?;

    let relationship = relationship.ok_or(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })?;

    Ok((link, relationship))
}

/// Finds all join-table records for a given coaching session.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_by_coaching_session_id(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Result<Vec<Model>, Error> {
    debug!("Finding goals linked to session {coaching_session_id}");

    Ok(Entity::find()
        .filter(Column::CoachingSessionId.eq(coaching_session_id))
        .all(db)
        .await?)
}

/// Finds all goal models linked to a given coaching session by eager-loading
/// through the join table.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_goals_by_coaching_session_id(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Result<Vec<goals::Model>, Error> {
    debug!("Finding goal models linked to session {coaching_session_id}");

    let links_with_goals = Entity::find()
        .filter(Column::CoachingSessionId.eq(coaching_session_id))
        .find_also_related(goals::Entity)
        .all(db)
        .await?;

    Ok(links_with_goals
        .into_iter()
        .filter_map(|(_, goal)| goal)
        .collect())
}

/// Returns up to [`super::goal::max_in_progress_goals`] in-progress goals
/// linked to a coaching session.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_in_progress_goals_by_coaching_session_id(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Result<Vec<goals::Model>, Error> {
    let all_goals = find_goals_by_coaching_session_id(db, coaching_session_id).await?;

    // Defensive cap: the write path enforces the limit, but we cap here too for safety.
    Ok(all_goals
        .into_iter()
        .filter(|g| g.in_progress())
        .take(super::goal::max_in_progress_goals())
        .collect())
}

/// Links all in-progress goals from a coaching relationship to a session.
///
/// Queries for goals with `InProgress` status on the given relationship,
/// then creates a join table record for each one.
/// Returns the number of goals linked.
///
/// This function does not manage its own transaction — callers are expected
/// to wrap it in a transaction when atomicity with other operations is needed.
///
/// # Errors
///
/// Returns `Error` if any database query or insert fails.
pub async fn link_in_progress_goals_to_session(
    db: &impl ConnectionTrait,
    coaching_relationship_id: Id,
    session_id: Id,
) -> Result<usize, Error> {
    // Defensive cap: the write path enforces the limit, but we cap here too for safety.
    let in_progress_goals: Vec<_> =
        super::goal::find_in_progress_goals_by_coaching_relationship_id(
            db,
            coaching_relationship_id,
        )
        .await?
        .into_iter()
        .take(super::goal::max_in_progress_goals())
        .collect();

    if in_progress_goals.is_empty() {
        return Ok(0);
    }

    let now = chrono::Utc::now();
    for g in &in_progress_goals {
        insert_link_row(db, session_id, g.id, now).await?;
    }

    Ok(in_progress_goals.len())
}

/// Finds all goal models for multiple coaching sessions at once, grouped by session ID.
///
/// Returns a `HashMap` where each key is a session ID and each value is the list
/// of goal models linked to that session. Sessions with no linked goals are not
/// included in the map.
///
/// This is the batch equivalent of [`find_goals_by_coaching_session_id`] — one query
/// replaces N individual calls, avoiding connection-pool exhaustion under concurrent load.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_goals_grouped_by_session_ids(
    db: &impl ConnectionTrait,
    session_ids: &[Id],
) -> Result<HashMap<Id, Vec<goals::Model>>, Error> {
    debug!("Batch loading goals for {} sessions", session_ids.len());

    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let links_with_goals = Entity::find()
        .filter(Column::CoachingSessionId.is_in(session_ids.iter().copied()))
        .find_also_related(goals::Entity)
        .all(db)
        .await?;

    let mut map: HashMap<Id, Vec<goals::Model>> = HashMap::new();
    for (link, goal_opt) in links_with_goals {
        if let Some(goal) = goal_opt {
            map.entry(link.coaching_session_id).or_default().push(goal);
        }
    }

    Ok(map)
}

/// Finds all session IDs belonging to a coaching relationship.
///
/// Used by the batch session-goals endpoint when the caller specifies
/// `coaching_relationship_id` instead of explicit session IDs.
///
/// # Errors
///
/// Returns `Error` if the database query fails.
pub async fn find_session_ids_by_coaching_relationship_id(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
) -> Result<Vec<Id>, Error> {
    debug!("Finding session IDs for coaching relationship {coaching_relationship_id}");

    let sessions = coaching_sessions::Entity::find()
        .filter(coaching_sessions::Column::CoachingRelationshipId.eq(coaching_relationship_id))
        .all(db)
        .await?;

    Ok(sessions.into_iter().map(|s| s.id).collect())
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

    fn build_link(coaching_session_id: Id, goal_id: Id) -> Model {
        let now = chrono::Utc::now();
        Model {
            id: Id::new_v4(),
            coaching_session_id,
            goal_id,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    #[tokio::test]
    async fn create_with_in_progress_goal_just_links_no_promotion() -> Result<(), Error> {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let goal = create_test_goal(relationship_id, Status::InProgress);
        let link = build_link(session_id, goal.id);

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // SELECT goal by id (lookup in create)
            .append_query_results(vec![vec![goal.clone()]])
            // SELECT existing link (duplicate check) — empty
            .append_query_results(vec![Vec::<Model>::new()])
            // INSERT join row
            .append_query_results(vec![vec![link.clone()]])
            .into_connection();

        let (result_link, promoted) = create(&db, session_id, goal.id).await?;

        assert_eq!(result_link.coaching_session_id, session_id);
        assert_eq!(result_link.goal_id, goal.id);
        assert!(
            promoted.is_none(),
            "InProgress goal should not be promoted again"
        );

        Ok(())
    }

    #[tokio::test]
    async fn create_with_not_started_goal_promotes_to_in_progress() -> Result<(), Error> {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let goal = create_test_goal(relationship_id, Status::NotStarted);
        let link = build_link(session_id, goal.id);
        let promoted = goals::Model {
            status: Status::InProgress,
            ..goal.clone()
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // SELECT goal by id (lookup in create)
            .append_query_results(vec![vec![goal.clone()]])
            // SELECT existing link (duplicate check) — empty
            .append_query_results(vec![Vec::<Model>::new()])
            // SELECT in-progress goals on relationship (cap check) — empty, under cap
            .append_query_results(vec![Vec::<goals::Model>::new()])
            // INSERT join row
            .append_query_results(vec![vec![link.clone()]])
            // UPDATE goal — promotion to InProgress
            .append_query_results(vec![vec![promoted.clone()]])
            .into_connection();

        let (result_link, promoted_goal) = create(&db, session_id, goal.id).await?;

        assert_eq!(result_link.goal_id, goal.id);
        let promoted_goal = promoted_goal.expect("NotStarted goal should be promoted");
        assert_eq!(promoted_goal.status, Status::InProgress);
        assert_eq!(promoted_goal.id, goal.id);

        Ok(())
    }

    #[tokio::test]
    async fn create_with_on_hold_goal_promotes_to_in_progress() -> Result<(), Error> {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let goal = create_test_goal(relationship_id, Status::OnHold);
        let link = build_link(session_id, goal.id);
        let promoted = goals::Model {
            status: Status::InProgress,
            ..goal.clone()
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![goal.clone()]])
            .append_query_results(vec![Vec::<Model>::new()])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .append_query_results(vec![vec![link.clone()]])
            .append_query_results(vec![vec![promoted.clone()]])
            .into_connection();

        let (_, promoted_goal) = create(&db, session_id, goal.id).await?;
        let promoted_goal = promoted_goal.expect("OnHold goal should be promoted");
        assert_eq!(promoted_goal.status, Status::InProgress);

        Ok(())
    }

    #[tokio::test]
    async fn create_with_completed_goal_returns_cannot_link_completed_goal() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let goal = create_test_goal(relationship_id, Status::Completed);

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // Only the goal lookup runs — we error before any writes
            .append_query_results(vec![vec![goal.clone()]])
            .into_connection();

        let result = create(&db, session_id, goal.id).await;
        let err = result.expect_err("linking a Completed goal should fail");
        assert!(
            matches!(err.error_kind, EntityApiErrorKind::CannotLinkCompletedGoal),
            "expected CannotLinkCompletedGoal, got {:?}",
            err.error_kind
        );
    }

    #[tokio::test]
    async fn create_with_wont_do_goal_returns_cannot_link_completed_goal() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let goal = create_test_goal(relationship_id, Status::WontDo);

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![goal.clone()]])
            .into_connection();

        let result = create(&db, session_id, goal.id).await;
        let err = result.expect_err("linking a WontDo goal should fail");
        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::CannotLinkCompletedGoal
        ));
    }

    #[tokio::test]
    async fn create_with_already_linked_goal_returns_goal_already_linked_to_session() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let goal = create_test_goal(relationship_id, Status::InProgress);
        let existing_link = build_link(session_id, goal.id);

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // SELECT goal by id
            .append_query_results(vec![vec![goal.clone()]])
            // SELECT existing link (duplicate check) — returns the existing row
            .append_query_results(vec![vec![existing_link.clone()]])
            // No further queries: error returns before INSERT
            .into_connection();

        let result = create(&db, session_id, goal.id).await;
        let err = result.expect_err("linking an already-linked goal should fail");
        assert!(
            matches!(
                err.error_kind,
                EntityApiErrorKind::GoalAlreadyLinkedToSession
            ),
            "expected GoalAlreadyLinkedToSession, got {:?}",
            err.error_kind
        );
    }

    #[tokio::test]
    async fn create_promotion_at_cap_returns_validation_error() {
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let target = create_test_goal(relationship_id, Status::NotStarted);
        let cap_goals = vec![
            create_test_goal(relationship_id, Status::InProgress),
            create_test_goal(relationship_id, Status::InProgress),
            create_test_goal(relationship_id, Status::InProgress),
        ];

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // SELECT goal by id (lookup in create)
            .append_query_results(vec![vec![target.clone()]])
            // SELECT existing link (duplicate check) — empty
            .append_query_results(vec![Vec::<Model>::new()])
            // SELECT in-progress goals on relationship (cap check) — at cap
            .append_query_results(vec![cap_goals])
            .into_connection();

        let result = create(&db, session_id, target.id).await;
        let err = result.expect_err("promoting past the cap should fail");
        assert!(
            matches!(err.error_kind, EntityApiErrorKind::ValidationError { .. }),
            "expected ValidationError (cap), got {:?}",
            err.error_kind
        );
    }

    #[tokio::test]
    async fn find_by_coaching_session_id_returns_linked_goals() -> Result<(), Error> {
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

        let results = find_by_coaching_session_id(&db, session_id).await?;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].coaching_session_id, session_id);

        Ok(())
    }

    #[tokio::test]
    async fn find_goals_by_coaching_session_id_returns_goal_models() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let session_id = Id::new_v4();
        let link_id = Id::new_v4();
        let goal_id = Id::new_v4();

        let link = Model {
            id: link_id,
            coaching_session_id: session_id,
            goal_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let goal = goals::Model {
            id: goal_id,
            coaching_relationship_id: Id::new_v4(),
            created_in_session_id: Some(session_id),
            user_id: Id::new_v4(),
            title: Some("Test Goal".to_string()),
            body: Some("Goal body".to_string()),
            status: entity::status::Status::InProgress,
            status_changed_at: Some(now.into()),
            completed_at: None,
            target_date: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![(link.clone(), Some(goal.clone()))]])
            .into_connection();

        let results = find_goals_by_coaching_session_id(&db, session_id).await?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, goal_id);
        assert_eq!(results[0].title, Some("Test Goal".to_string()));

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

    fn create_test_goal(
        coaching_relationship_id: Id,
        status: entity::status::Status,
    ) -> goals::Model {
        let now = chrono::Utc::now().fixed_offset();
        goals::Model {
            id: Id::new_v4(),
            coaching_relationship_id,
            created_in_session_id: None,
            user_id: Id::new_v4(),
            title: Some("Test goal".to_string()),
            body: None,
            status,
            status_changed_at: None,
            completed_at: None,
            target_date: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn link_in_progress_goals_to_session_links_goals() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let session_id = Id::new_v4();
        let goal1 = create_test_goal(relationship_id, entity::status::Status::InProgress);
        let goal2 = create_test_goal(relationship_id, entity::status::Status::InProgress);

        let now = chrono::Utc::now();
        let join1 = Model {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            goal_id: goal1.id,
            created_at: now.into(),
            updated_at: now.into(),
        };
        let join2 = Model {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            goal_id: goal2.id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        // Mock sequence: 1 SELECT (in-progress goals) + 2 INSERTs (one per goal)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![goal1, goal2]])
            .append_query_results(vec![vec![join1]])
            .append_query_results(vec![vec![join2]])
            .into_connection();

        let count = link_in_progress_goals_to_session(&db, relationship_id, session_id).await?;
        assert_eq!(count, 2, "should link 2 in-progress goals");

        Ok(())
    }

    #[tokio::test]
    async fn link_in_progress_goals_to_session_skips_when_none() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let session_id = Id::new_v4();

        // Mock: SELECT returns empty vec — no INSERTs expected
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let count = link_in_progress_goals_to_session(&db, relationship_id, session_id).await?;
        assert_eq!(count, 0, "should link 0 goals when none are in-progress");

        Ok(())
    }

    #[tokio::test]
    async fn find_goals_grouped_by_session_ids_returns_grouped_goals() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let relationship_id = Id::new_v4();
        let session_a = Id::new_v4();
        let session_b = Id::new_v4();
        let goal1 = create_test_goal(relationship_id, entity::status::Status::InProgress);
        let goal2 = create_test_goal(relationship_id, entity::status::Status::NotStarted);

        // Session A has goal1, Session B has goal2
        let link1 = Model {
            id: Id::new_v4(),
            coaching_session_id: session_a,
            goal_id: goal1.id,
            created_at: now.into(),
            updated_at: now.into(),
        };
        let link2 = Model {
            id: Id::new_v4(),
            coaching_session_id: session_b,
            goal_id: goal2.id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![
                (link1, Some(goal1.clone())),
                (link2, Some(goal2.clone())),
            ]])
            .into_connection();

        let result = find_goals_grouped_by_session_ids(&db, &[session_a, session_b]).await?;

        assert_eq!(result.len(), 2, "should have 2 session entries");
        assert_eq!(result[&session_a].len(), 1);
        assert_eq!(result[&session_a][0].id, goal1.id);
        assert_eq!(result[&session_b].len(), 1);
        assert_eq!(result[&session_b][0].id, goal2.id);

        Ok(())
    }

    #[tokio::test]
    async fn find_goals_grouped_by_session_ids_returns_empty_for_empty_input() -> Result<(), Error>
    {
        // No mock needed — function returns early for empty slice
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let result = find_goals_grouped_by_session_ids(&db, &[]).await?;

        assert!(result.is_empty(), "should return empty map for empty input");

        Ok(())
    }

    #[tokio::test]
    async fn find_goals_grouped_by_session_ids_groups_multiple_goals_per_session(
    ) -> Result<(), Error> {
        let now = chrono::Utc::now();
        let relationship_id = Id::new_v4();
        let session_id = Id::new_v4();
        let goal1 = create_test_goal(relationship_id, entity::status::Status::InProgress);
        let goal2 = create_test_goal(relationship_id, entity::status::Status::InProgress);

        let link1 = Model {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            goal_id: goal1.id,
            created_at: now.into(),
            updated_at: now.into(),
        };
        let link2 = Model {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            goal_id: goal2.id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![
                (link1, Some(goal1.clone())),
                (link2, Some(goal2.clone())),
            ]])
            .into_connection();

        let result = find_goals_grouped_by_session_ids(&db, &[session_id]).await?;

        assert_eq!(result.len(), 1, "should have 1 session entry");
        assert_eq!(
            result[&session_id].len(),
            2,
            "should have 2 goals for the session"
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_session_ids_by_coaching_relationship_id_returns_ids() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let relationship_id = Id::new_v4();
        let session1_id = Id::new_v4();
        let session2_id = Id::new_v4();

        let session1 = entity::coaching_sessions::Model {
            id: session1_id,
            coaching_relationship_id: relationship_id,
            collab_document_name: None,
            date: now.naive_utc(),
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
        };
        let session2 = entity::coaching_sessions::Model {
            id: session2_id,
            coaching_relationship_id: relationship_id,
            collab_document_name: None,
            date: now.naive_utc(),
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![session1, session2]])
            .into_connection();

        let result = find_session_ids_by_coaching_relationship_id(&db, relationship_id).await?;

        assert_eq!(result.len(), 2);
        assert!(result.contains(&session1_id));
        assert!(result.contains(&session2_id));

        Ok(())
    }
}
