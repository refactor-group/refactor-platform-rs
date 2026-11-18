//! Exercises the production ON CONFLICT DO NOTHING insert behind creating a coaching
//! relationship against a real SQL engine, so a conflict's outcome is what SeaORM reports,
//! the kind that creating one stores, and the organization membership rule of `is_coach_of` and a user's own relationship list.

use std::collections::BTreeSet;

use chrono::Utc;
use entity::coaching_relationship_kind::Kind;
use entity::coaching_relationships::{ActiveModel, Entity, Visibility};
use entity::Id;
use entity_api::coaching_relationship::{create, insert_unless_exists, Draft};
use entity_api::error::EntityApiErrorKind;
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Set};

use super::{find_by_user_id_with_user_names, is_coach_of, RoleFilterable};

use crate::test_utils::sqlite::read_back::relationships_between;
use crate::test_utils::sqlite::{
    self, seed_lapsed_membership, seed_membership, seed_organization, seed_user, seed_user_as,
    within_time_limit,
};

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
        kind: Set(Kind::Coaching),
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

/// The organization `o` with three of its members.
async fn three_members() -> (DatabaseConnection, Id, [Id; 3]) {
    let db = sqlite::database().await;
    sqlite::mirror_migration_defaults(&db).await;
    let o = seed_organization(&db).await;
    let members = [
        seed_user(&db).await,
        seed_user(&db).await,
        seed_user(&db).await,
    ];
    for user in members {
        seed_membership(&db, user, o).await;
    }
    (db, o, members)
}

fn draft(coach_id: Id, coachee_id: Id, kind: Kind) -> Draft {
    Draft {
        coach_id,
        coachee_id,
        kind,
    }
}

#[tokio::test]
async fn create_stores_the_kind_it_is_given() {
    within_time_limit(async {
        let (db, o, [coach, colleague, coachee]) = three_members().await;

        for (other, kind) in [(colleague, Kind::Colleague), (coachee, Kind::Coaching)] {
            create(&db, o, draft(coach, other, kind))
                .await
                .expect("the relationship is created");
            let kinds: Vec<Kind> = relationships_between(&db, coach, other)
                .await
                .into_iter()
                .map(|relationship| relationship.kind)
                .collect();
            assert_eq!(kinds, vec![kind], "given {kind:?}");
        }
    })
    .await
}

#[tokio::test]
async fn create_returns_the_existing_relationship_of_the_same_kind() {
    within_time_limit(async {
        let (db, o, [coach, coachee, _]) = three_members().await;
        let first = create(&db, o, draft(coach, coachee, Kind::Coaching))
            .await
            .expect("the relationship is created");

        let again = create(&db, o, draft(coach, coachee, Kind::Coaching))
            .await
            .expect("the existing relationship is returned");

        assert_eq!(again.id, first.id);
        assert_eq!(relationships_between(&db, coach, coachee).await.len(), 1);
    })
    .await
}

/// KIND-27
#[tokio::test]
async fn a_returned_colleague_relationship_names_each_seat_by_its_own_user() {
    within_time_limit(async {
        let (db, o, _) = three_members().await;
        let (a, b) = (Id::new_v4(), Id::new_v4());
        for (id, name) in [(a, ("Ada", "Lovelace")), (b, ("Grace", "Hopper"))] {
            seed_user_as(&db, id, format!("{id}@example.com"), name, None).await;
            seed_membership(&db, id, o).await;
        }
        let first = create(&db, o, draft(a, b, Kind::Colleague))
            .await
            .expect("the relationship is created");

        let again = create(&db, o, draft(b, a, Kind::Colleague))
            .await
            .expect("the existing relationship is returned");

        assert_eq!(again.id, first.id);
        assert_eq!((again.coach_id, again.coachee_id), (a, b));
        assert_eq!(
            (
                again.coach_first_name.as_str(),
                again.coach_last_name.as_str()
            ),
            ("Ada", "Lovelace")
        );
        assert_eq!(
            (
                again.coachee_first_name.as_str(),
                again.coachee_last_name.as_str()
            ),
            ("Grace", "Hopper")
        );
    })
    .await
}

/// KIND-45
#[tokio::test]
async fn a_returned_colleague_relationship_carries_each_seats_own_email() {
    within_time_limit(async {
        let (db, o, _) = three_members().await;
        let (a, b) = (Id::new_v4(), Id::new_v4());
        for id in [a, b] {
            seed_user_as(&db, id, format!("{id}@example.com"), ("Sam", "Lee"), None).await;
            seed_membership(&db, id, o).await;
        }
        create(&db, o, draft(a, b, Kind::Colleague))
            .await
            .expect("the relationship is created");

        let again = create(&db, o, draft(b, a, Kind::Colleague))
            .await
            .expect("the existing relationship is returned");

        assert_eq!(
            (again.coach_email, again.coachee_email),
            (format!("{a}@example.com"), format!("{b}@example.com"))
        );
    })
    .await
}

#[tokio::test]
async fn create_refuses_a_pair_whose_relationship_is_of_another_kind() {
    within_time_limit(async {
        let (db, o, [coach, coachee, _]) = three_members().await;
        create(&db, o, draft(coach, coachee, Kind::Coaching))
            .await
            .expect("the relationship is created");
        let before = relationships_between(&db, coach, coachee).await;

        let error = create(&db, o, draft(coach, coachee, Kind::Colleague))
            .await
            .expect_err("another kind must not reuse the pair's relationship");

        assert_eq!(
            error.error_kind,
            EntityApiErrorKind::PairAlreadyRelated {
                relationship_id: before[0].id
            }
        );
        assert_eq!(relationships_between(&db, coach, coachee).await, before);
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].kind, Kind::Coaching);
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

/// Every relationship, as coach or coachee.
struct AnyRole;

impl RoleFilterable for AnyRole {
    fn filter_coach_only(&self) -> bool {
        false
    }

    fn filter_coachee_only(&self) -> bool {
        false
    }
}

/// The ids of the relationships `viewer` lists as their own.
async fn own_relationship_ids(db: &DatabaseConnection, viewer: &Visibility) -> BTreeSet<Id> {
    find_by_user_id_with_user_names(db, viewer, AnyRole)
        .await
        .expect("the relationships are read")
        .into_iter()
        .map(|r| r.id)
        .collect()
}

/// M1-01
#[tokio::test]
async fn a_users_own_relationships_skip_an_org_they_left() {
    within_time_limit(async {
        let db = database().await;
        let m = seed_lapsed_membership(&db, Utc::now().naive_utc()).await;
        assert_eq!(
            own_relationship_ids(&db, &m.in_o1()).await,
            BTreeSet::from([m.relationships[0]])
        );
    })
    .await
}

/// M1-07, M1-08: a SuperAdmin with no roles, or a member of both orgs, sees both.
#[tokio::test]
async fn a_super_admin_or_a_member_of_both_orgs_sees_every_own_relationship() {
    within_time_limit(async {
        let db = database().await;
        let m = seed_lapsed_membership(&db, Utc::now().naive_utc()).await;
        for viewer in [m.super_admin(), m.in_both()] {
            assert_eq!(
                own_relationship_ids(&db, &viewer).await,
                BTreeSet::from(m.relationships),
                "{viewer:?}"
            );
        }
    })
    .await
}
