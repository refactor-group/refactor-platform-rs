//! `coach_is_member` / `coachee_is_member` on the organization's relationship list,
//! against a real SQL engine: which role rows count.

use chrono::Utc;
use entity::roles::Role;
use entity_api::user_role::retain_organization_members;
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
    seed_role(db, user_id, Role::Admin, Some(organization_id)).await
}

/// Gives `user_id` `role` in `organization_id` (`None`: a global role).
async fn seed_role(db: &DatabaseConnection, user_id: Id, role: Role, organization_id: Option<Id>) {
    let now = Utc::now();
    user_roles::ActiveModel {
        id: Set(Id::new_v4()),
        role: Set(role),
        organization_id: Set(organization_id),
        user_id: Set(user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the role is seeded");
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

        assert!(relationship.membership.coach_is_member, "{relationship:?}");
        assert!(
            relationship.membership.coachee_is_member,
            "{relationship:?}"
        );
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

        assert!(relationship.membership.coach_is_member, "{relationship:?}");
        assert!(
            !relationship.membership.coachee_is_member,
            "{relationship:?}"
        );
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

        assert!(!relationship.membership.coach_is_member, "{relationship:?}");
        assert!(
            relationship.membership.coachee_is_member,
            "{relationship:?}"
        );
    })
    .await
}

/// How the coachee stands in the relationship's organization, for M5-01.
#[derive(Debug, Clone, Copy)]
enum Standing {
    Member,
    Removed,
    ElsewhereOnly,
    GlobalSuperAdmin,
    OrganizationAdmin,
}

impl Standing {
    async fn seed(self, s: &Seeded) {
        match self {
            Standing::Member => seed_membership(&s.db, s.coachee, s.organization).await,
            Standing::Removed => {}
            Standing::ElsewhereOnly => {
                seed_membership(&s.db, s.coachee, seed_organization(&s.db).await).await
            }
            Standing::GlobalSuperAdmin => seed_role(&s.db, s.coachee, Role::SuperAdmin, None).await,
            Standing::OrganizationAdmin => seed_admin(&s.db, s.coachee, s.organization).await,
        }
    }

    fn is_member(self) -> bool {
        matches!(
            self,
            Standing::Member | Standing::GlobalSuperAdmin | Standing::OrganizationAdmin
        )
    }
}

#[tokio::test]
async fn m5_01_the_list_field_agrees_with_the_notify_filter() {
    within_time_limit(async {
        for standing in [
            Standing::Member,
            Standing::Removed,
            Standing::ElsewhereOnly,
            Standing::GlobalSuperAdmin,
            Standing::OrganizationAdmin,
        ] {
            let s = seeded().await;
            seed_membership(&s.db, s.coach, s.organization).await;
            standing.seed(&s).await;

            let listed = listed(&s, s.coach).await.membership.coachee_is_member;
            let retained = retain_organization_members(&s.db, &[s.coachee], s.organization)
                .await
                .expect("the filter reads")
                .contains(&s.coachee);

            assert_eq!(listed, retained, "{standing:?}");
            assert_eq!(listed, standing.is_member(), "{standing:?}");
        }
    })
    .await
}
