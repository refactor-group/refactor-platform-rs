//! Exercises the production ON CONFLICT DO NOTHING insert behind creating a coaching
//! relationship against a real SQL engine, so a conflict's outcome is what SeaORM reports,
//! and `is_coach_of`'s organization membership rule.

use std::collections::BTreeSet;

use chrono::Utc;
use entity::coaching_relationships::{ActiveModel, Entity, Visibility};
use entity::Id;
use entity_api::coaching_relationship::insert_unless_exists;
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Set};

use super::is_coach_of;

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

/// COACH coaches COACHEE, who coaches MENTEE, both in O1.
struct Coaching {
    db: DatabaseConnection,
    coach: Id,
    coachee: Id,
    mentee: Id,
    o1: Id,
}

async fn coaching() -> Coaching {
    let db = database().await;
    let (coach, coachee, mentee, o1) = (
        seed_user(&db).await,
        seed_user(&db).await,
        seed_user(&db).await,
        seed_organization(&db).await,
    );
    for (coach_id, coachee_id, slug) in [(coach, coachee, "r1"), (coachee, mentee, "r4")] {
        relationship(Id::new_v4(), coach_id, coachee_id, o1, slug)
            .insert(&db)
            .await
            .expect("the relationship is seeded");
    }
    Coaching {
        db,
        coach,
        coachee,
        mentee,
        o1,
    }
}

/// A non-SuperAdmin `user_id` holding a role in each of `organization_ids`.
fn member_of(user_id: Id, organization_ids: &[Id]) -> Visibility {
    Visibility {
        user_id,
        is_super_admin: false,
        admin_org_ids: vec![],
        member_org_ids: organization_ids.to_vec(),
    }
}

#[tokio::test]
async fn a_coach_in_the_relationships_org_is_its_coach() {
    within_time_limit(async {
        let c = coaching().await;
        let coach = member_of(c.coach, &[c.o1]);
        assert!(is_coach_of(&c.db, &coach, c.coachee)
            .await
            .expect("the query runs"));
    })
    .await
}

#[tokio::test]
async fn a_coach_in_no_org_is_not_its_coach() {
    within_time_limit(async {
        let c = coaching().await;
        let coach = member_of(c.coach, &[]);
        assert!(!is_coach_of(&c.db, &coach, c.coachee)
            .await
            .expect("the query runs"));
    })
    .await
}

#[tokio::test]
async fn a_super_admin_coach_with_no_org_role_is_its_coach() {
    within_time_limit(async {
        let c = coaching().await;
        let coach = Visibility {
            is_super_admin: true,
            ..member_of(c.coach, &[])
        };
        assert!(is_coach_of(&c.db, &coach, c.coachee)
            .await
            .expect("the query runs"));
    })
    .await
}

#[tokio::test]
async fn the_coachees_own_coachee_is_not_their_coach() {
    within_time_limit(async {
        let c = coaching().await;
        let mentee = member_of(c.mentee, &[c.o1]);
        assert!(!is_coach_of(&c.db, &mentee, c.coachee)
            .await
            .expect("the query runs"));
    })
    .await
}
