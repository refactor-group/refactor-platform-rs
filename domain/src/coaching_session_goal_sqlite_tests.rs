//! The ON CONFLICT DO NOTHING bulk insert that links in-progress goals to a session,
//! run against a real SQL engine so the returned rows are what SeaORM truly reports.

use std::collections::BTreeSet;

use chrono::Utc;
use entity::coaching_sessions_goals::{ActiveModel, Column, Entity};
use entity::{coaching_sessions, goals, status, Id};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Set};

use crate::test_utils::sqlite::{self, seed_coaching_session, within_time_limit};

/// The synced schema plus the session-goal links table's production unique index.
async fn database() -> DatabaseConnection {
    let db = sqlite::database().await;
    // Mirrors migration/src/m20260309_000000_goal_relationship_scoping.rs.
    db.execute_unprepared(
        "CREATE UNIQUE INDEX refactor_platform.coaching_sessions_goals_session_goal_unique \
         ON coaching_sessions_goals (coaching_session_id, goal_id)",
    )
    .await
    .expect("the unique index is created");
    db
}

/// The clause `entity_api::coaching_session_goal::link_in_progress_goals_to_session` builds.
fn conflict() -> OnConflict {
    OnConflict::columns([Column::CoachingSessionId, Column::GoalId])
        .do_nothing()
        .to_owned()
}

/// Inserts a goal in the relationship of `coaching_session_id` and returns its id.
async fn seed_goal(db: &DatabaseConnection, coaching_session_id: Id, user_id: Id) -> Id {
    let session = coaching_sessions::Entity::find_by_id(coaching_session_id)
        .one(db)
        .await
        .expect("the session is read")
        .expect("the session exists");
    let now = Utc::now();
    goals::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_relationship_id: Set(session.coaching_relationship_id),
        created_in_session_id: Set(None),
        user_id: Set(user_id),
        title: Set(None),
        body: Set(None),
        status: Set(status::Status::InProgress),
        status_changed_at: Set(None),
        completed_at: Set(None),
        target_date: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the goal is seeded")
    .id
}

fn link(coaching_session_id: Id, goal_id: Id) -> ActiveModel {
    let now = Utc::now();
    ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(coaching_session_id),
        goal_id: Set(goal_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
}

#[tokio::test]
async fn returning_many_yields_only_the_rows_written() {
    within_time_limit(async {
        let db = database().await;
        let (s, user) = seed_coaching_session(&db).await;
        let (g1, g2, g3) = (
            seed_goal(&db, s, user).await,
            seed_goal(&db, s, user).await,
            seed_goal(&db, s, user).await,
        );

        let first = Entity::insert_many([link(s, g1), link(s, g2)])
            .on_conflict(conflict())
            .exec_with_returning(&db)
            .await
            .expect("both links are written");
        assert_eq!(first.len(), 2);
        let goals: BTreeSet<Id> = first.into_iter().map(|row| row.goal_id).collect();
        assert_eq!(goals, BTreeSet::from([g1, g2]));

        let second = Entity::insert_many([link(s, g2), link(s, g3)])
            .on_conflict(conflict())
            .exec_with_returning(&db)
            .await
            .expect("the new link is written");
        assert_eq!(second.len(), 1, "only the non-conflicting row comes back");
        assert_eq!(second[0].goal_id, g3);

        let rows = Entity::find().all(&db).await.expect("the table is read");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().filter(|row| row.goal_id == g2).count(), 1);
    })
    .await
}

#[tokio::test]
async fn returning_many_when_every_row_conflicts() {
    within_time_limit(async {
        let db = database().await;
        let (s, user) = seed_coaching_session(&db).await;
        let g1 = seed_goal(&db, s, user).await;

        Entity::insert_many([link(s, g1)])
            .on_conflict(conflict())
            .exec_with_returning(&db)
            .await
            .expect("the link is written");

        let result = Entity::insert_many([link(s, g1)])
            .on_conflict(conflict())
            .exec_with_returning(&db)
            .await
            .expect("a fully conflicting batch is not an error");
        // A batch where every row conflicts reports Ok with no rows.
        assert!(result.is_empty(), "nothing was written, got {result:?}");

        let rows = Entity::find().all(&db).await.expect("the table is read");
        assert_eq!(rows.len(), 1);
    })
    .await
}
