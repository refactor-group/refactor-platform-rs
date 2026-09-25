//! Exercises the production ON CONFLICT DO NOTHING insert behind creating a coaching
//! relationship against a real SQL engine, so a conflict's outcome is what SeaORM reports.

use std::collections::BTreeSet;

use chrono::Utc;
use entity::coaching_relationships::{ActiveModel, Entity};
use entity::Id;
use entity_api::coaching_relationship::insert_unless_exists;
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait, Set};

use crate::test_utils::sqlite::{self, seed_organization, seed_user, within_time_limit};

/// The synced schema plus the relationships table's production unique index.
async fn database() -> DatabaseConnection {
    let db = sqlite::database().await;
    // Mirrors migration/src/base_refactor_platform_rs.sql.
    db.execute_unprepared(
        "CREATE UNIQUE INDEX refactor_platform.coaching_relationships_coach_coachee_org \
         ON coaching_relationships (coach_id, coachee_id, organization_id)",
    )
    .await
    .expect("the unique index is created");
    db
}

fn relationship(
    id: Id,
    coach_id: Id,
    coachee_id: Id,
    organization_id: Id,
    slug: &str,
) -> ActiveModel {
    let now = Utc::now();
    ActiveModel {
        id: Set(id),
        organization_id: Set(organization_id),
        coach_id: Set(coach_id),
        coachee_id: Set(coachee_id),
        slug: Set(slug.to_string()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
}

#[tokio::test]
async fn a_conflicting_insert_writes_nothing_and_reports_none() {
    within_time_limit(async {
        let db = database().await;
        let (a, b) = (Id::new_v4(), Id::new_v4());
        let (c, e, o) = (
            seed_user(&db).await,
            seed_user(&db).await,
            seed_organization(&db).await,
        );

        let first = insert_unless_exists(&db, relationship(a, c, e, o, "first"))
            .await
            .expect("the first insert succeeds")
            .expect("the first insert is written");
        assert_eq!(first.id, a);

        let result = insert_unless_exists(&db, relationship(b, c, e, o, "second")).await;
        assert!(
            matches!(result, Ok(None)),
            "a conflict reports Ok(None), got {result:?}"
        );

        let rows = Entity::find().all(&db).await.expect("the table is read");
        assert_eq!(rows.len(), 1, "the conflicting insert wrote nothing");
        assert_eq!(rows[0].id, a);
        assert_eq!(rows[0].slug, "first");
    })
    .await
}

#[tokio::test]
async fn an_insert_outside_the_conflict_target_is_written() {
    within_time_limit(async {
        let db = database().await;
        let (a, other) = (Id::new_v4(), Id::new_v4());
        let (c, e, other_coachee, o) = (
            seed_user(&db).await,
            seed_user(&db).await,
            seed_user(&db).await,
            seed_organization(&db).await,
        );

        insert_unless_exists(&db, relationship(a, c, e, o, "first"))
            .await
            .expect("the first insert succeeds")
            .expect("the first insert is written");
        insert_unless_exists(&db, relationship(other, c, other_coachee, o, "second"))
            .await
            .expect("the second insert succeeds")
            .expect("a different coachee is written");

        let rows = Entity::find().all(&db).await.expect("the table is read");
        assert_eq!(rows.len(), 2, "both inserts are written");
        let ids: BTreeSet<Id> = rows.into_iter().map(|row| row.id).collect();
        assert_eq!(ids, BTreeSet::from([a, other]));
    })
    .await
}
