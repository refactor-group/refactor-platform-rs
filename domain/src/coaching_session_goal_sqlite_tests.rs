//! The ON CONFLICT DO NOTHING bulk insert that links in-progress goals to a session,
//! run against a real SQL engine so the returned rows are what SeaORM truly reports.

use std::collections::BTreeSet;

use chrono::Utc;
use entity::coaching_sessions_goals::{ActiveModel, Column, Entity};
use entity::Id;
use sea_orm::sea_query::OnConflict;
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, Set};

use crate::test_utils::sqlite::{create_table, empty_database, within_time_limit};

/// A database holding the session-goal links table and its production unique index.
async fn database() -> DatabaseConnection {
    let db = empty_database().await;
    create_table(&db, Entity).await;
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
        let (s, g1, g2, g3) = (Id::new_v4(), Id::new_v4(), Id::new_v4(), Id::new_v4());

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
        let (s, g1) = (Id::new_v4(), Id::new_v4());

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
        // SeaORM 1.1 reports a batch where every row conflicts as Ok with no rows.
        assert!(result.is_empty(), "nothing was written, got {result:?}");

        let rows = Entity::find().all(&db).await.expect("the table is read");
        assert_eq!(rows.len(), 1);
    })
    .await
}
