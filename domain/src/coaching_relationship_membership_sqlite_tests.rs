//! `coach_is_member` / `coachee_is_member` on the organization's relationship list,
//! against a real SQL engine: which role rows count.

use chrono::Utc;
use entity::roles::Role;
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};

use super::{find_by_organization_for_user_with_user_names, CoachingRelationshipWithUserNames};
use crate::test_utils::sqlite::{
    database, seed_membership, seed_organization, seed_user, within_time_limit,
};
use crate::{coaching_relationships, user_roles, Id};

/// One relationship in O; who holds a role there is up to each test.
struct Seeded {
    db: DatabaseConnection,
    coach: Id,
    coachee: Id,
    organization: Id,
}

async fn seeded() -> Seeded {
    let db = database().await;
    let (coach, coachee, organization) = (
        seed_user(&db).await,
        seed_user(&db).await,
        seed_organization(&db).await,
    );
    let now = Utc::now();
    coaching_relationships::ActiveModel {
        id: Set(Id::new_v4()),
        organization_id: Set(organization),
        coach_id: Set(coach),
        coachee_id: Set(coachee),
        slug: Set("membership".to_string()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(&db)
    .await
    .expect("the relationship is seeded");
    Seeded {
        db,
        coach,
        coachee,
        organization,
    }
}

/// Gives `user_id` an Admin role in `organization_id`.
async fn seed_admin(db: &DatabaseConnection, user_id: Id, organization_id: Id) {
    let now = Utc::now();
    user_roles::ActiveModel {
        id: Set(Id::new_v4()),
        role: Set(Role::Admin),
        organization_id: Set(Some(organization_id)),
        user_id: Set(user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the admin role is seeded");
}

/// `caller`'s view of the organization's one relationship.
async fn listed(s: &Seeded, caller: Id) -> CoachingRelationshipWithUserNames {
    let mut listed = find_by_organization_for_user_with_user_names(&s.db, caller, s.organization)
        .await
        .expect("the list reads");
    assert_eq!(listed.len(), 1, "{listed:?}");
    listed.remove(0)
}

#[tokio::test]
async fn m4_01_both_members_are_marked_members() {
    within_time_limit(async {
        let s = seeded().await;
        seed_membership(&s.db, s.coach, s.organization).await;
        seed_membership(&s.db, s.coachee, s.organization).await;

        let relationship = listed(&s, s.coach).await;

        assert!(relationship.coach_is_member, "{relationship:?}");
        assert!(relationship.coachee_is_member, "{relationship:?}");
    })
    .await
}

#[tokio::test]
async fn m4_02_a_removed_coachee_is_not_a_member() {
    within_time_limit(async {
        let s = seeded().await;
        seed_membership(&s.db, s.coach, s.organization).await;
        // A role elsewhere does not count.
        seed_membership(&s.db, s.coachee, seed_organization(&s.db).await).await;

        let relationship = listed(&s, s.coach).await;

        assert!(relationship.coach_is_member, "{relationship:?}");
        assert!(!relationship.coachee_is_member, "{relationship:?}");
    })
    .await
}

#[tokio::test]
async fn m4_03_a_removed_coach_is_not_a_member_to_an_admin() {
    within_time_limit(async {
        let s = seeded().await;
        seed_membership(&s.db, s.coachee, s.organization).await;
        let admin = seed_user(&s.db).await;
        seed_admin(&s.db, admin, s.organization).await;

        let relationship = listed(&s, admin).await;

        assert!(!relationship.coach_is_member, "{relationship:?}");
        assert!(relationship.coachee_is_member, "{relationship:?}");
    })
    .await
}
