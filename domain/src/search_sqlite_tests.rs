//! Pins search's relationship visibility to direct access. `visible_to` (the SQL
//! form search scopes on) and `grants_access_to` (the in-memory check every
//! direct read goes through) are two expressions of one rule; this runs the real
//! query over an access matrix and requires them to agree on every pair.

use chrono::Utc;
use entity::{coaching_relationships, user_roles, users, Id};
use entity_api::search::resolve_visible_relationship_ids;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use super::visibility_for;
use crate::test_utils::sqlite::{self, seed_organization, seed_user, within_time_limit};
use crate::users::Role;

async fn seed_relationship(
    db: &DatabaseConnection,
    coach_id: Id,
    coachee_id: Id,
    organization_id: Id,
) -> coaching_relationships::Model {
    let id = Id::new_v4();
    let now = Utc::now();
    coaching_relationships::ActiveModel {
        id: Set(id),
        organization_id: Set(organization_id),
        coach_id: Set(coach_id),
        coachee_id: Set(coachee_id),
        slug: Set(format!("relationship-{id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the relationship is seeded")
}

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

/// The user with `roles` populated, as `AuthenticatedUser` hands it over.
async fn load_user(db: &DatabaseConnection, id: Id) -> users::Model {
    let mut user = users::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the user loads")
        .expect("the user exists");
    user.roles = user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(id))
        .all(db)
        .await
        .expect("the roles load");
    user
}

#[tokio::test]
async fn search_visibility_matches_grants_access_to_for_every_user_and_relationship() {
    within_time_limit(async {
        let db = sqlite::database().await;
        let (org_a, org_b) = (seed_organization(&db).await, seed_organization(&db).await);

        let coach = seed_user(&db).await;
        let removed_coachee = seed_user(&db).await;
        let org_admin = seed_user(&db).await;
        let bystander = seed_user(&db).await;
        let wrong_org_coachee = seed_user(&db).await;
        let two_org_coach = seed_user(&db).await;

        // Coach is a member of A only; the coachee was removed from A and holds no role.
        seed_role(&db, coach, Role::User, Some(org_a)).await;
        // Admin of A who coaches one A relationship: sees it, but the Admin role
        // adds nothing for the other A relationships.
        seed_role(&db, org_admin, Role::Admin, Some(org_a)).await;
        // Member of A whose only relationship is in B.
        seed_role(&db, bystander, Role::User, Some(org_a)).await;
        // Coachee in an A and a B relationship, but a member of B only.
        seed_role(&db, wrong_org_coachee, Role::User, Some(org_b)).await;
        // Coaches in both organizations and is a member of both.
        seed_role(&db, two_org_coach, Role::User, Some(org_a)).await;
        seed_role(&db, two_org_coach, Role::Admin, Some(org_b)).await;

        let relationships = vec![
            seed_relationship(&db, coach, removed_coachee, org_a).await,
            seed_relationship(&db, org_admin, wrong_org_coachee, org_a).await,
            seed_relationship(&db, two_org_coach, coach, org_a).await,
            seed_relationship(&db, two_org_coach, wrong_org_coachee, org_b).await,
            seed_relationship(&db, two_org_coach, bystander, org_b).await,
        ];

        let mut agreed_grants = 0;
        for user_id in [
            coach,
            removed_coachee,
            org_admin,
            bystander,
            wrong_org_coachee,
            two_org_coach,
        ] {
            let user = load_user(&db, user_id).await;
            let visible = resolve_visible_relationship_ids(&db, &visibility_for(&user), None, None)
                .await
                .expect("visibility resolves")
                .expect("a non-super-admin always gets an explicit set");

            for relationship in &relationships {
                let granted = relationship.grants_access_to(&user);
                assert_eq!(
                    visible.contains(&relationship.id),
                    granted,
                    "search and direct access disagree for user {user_id} on relationship {} \
                     (org {}, coach {}, coachee {})",
                    relationship.id,
                    relationship.organization_id,
                    relationship.coach_id,
                    relationship.coachee_id,
                );
                agreed_grants += usize::from(granted);
            }
        }

        // Guards against a matrix that agrees only because nobody sees anything:
        // the coach on two A relationships, the org admin on one, the two-org coach
        // on all three, and the wrong-org coachee on its B relationship only.
        assert_eq!(agreed_grants, 7);
    })
    .await;
}

/// Super admins deliberately skip `visible_to` and search everything, while
/// `grants_access_to` still requires them to participate. Pinned here so the
/// divergence stays a decision rather than an accident.
#[tokio::test]
async fn a_super_admin_searches_every_relationship_unrestricted() {
    within_time_limit(async {
        let db = sqlite::database().await;
        let super_admin = seed_user(&db).await;
        seed_role(&db, super_admin, Role::SuperAdmin, None).await;
        let user = load_user(&db, super_admin).await;

        let visible = resolve_visible_relationship_ids(&db, &visibility_for(&user), None, None)
            .await
            .expect("visibility resolves");

        assert_eq!(visible, None);
    })
    .await;
}
