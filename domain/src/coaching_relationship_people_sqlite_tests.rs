//! A relationship's coach and coachee, read together in one query against a real SQL engine.

use chrono::Utc;
use entity::{coaching_relationships, users};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};

use super::*;
use crate::test_utils::sqlite::{database, seed_organization, seed_user, within_time_limit};
use crate::Id;

async fn relationship(db: &DatabaseConnection, coach_id: Id, coachee_id: Id) -> Id {
    let id = Id::new_v4();
    let now = Utc::now();
    coaching_relationships::ActiveModel {
        id: Set(id),
        organization_id: Set(seed_organization(db).await),
        coach_id: Set(coach_id),
        coachee_id: Set(coachee_id),
        slug: Set(format!("relationship-{id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the relationship is seeded");
    id
}

/// Coach first, coachee second, however the rows come back.
#[tokio::test]
async fn coach_and_coachee_come_back_in_role_order() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let (coach_id, coachee_id) = (seed_user(&db).await, seed_user(&db).await);
        let mine = relationship(&db, coach_id, coachee_id).await;
        // The same two people with roles swapped, plus an unrelated pair, must not leak in.
        relationship(&db, coachee_id, coach_id).await;
        relationship(&db, seed_user(&db).await, seed_user(&db).await).await;

        let (coach, coachee): (users::Model, users::Model) =
            find_coach_and_coachee(&db, mine).await?;

        assert_eq!(coach.id, coach_id);
        assert_eq!(coachee.id, coachee_id);
        assert_eq!(coach.email, format!("{coach_id}@example.com"));
        assert_eq!(coachee.email, format!("{coachee_id}@example.com"));

        Ok::<(), Error>(())
    })
    .await
}

/// A relationship whose coach and coachee are the same user yields that user twice.
#[tokio::test]
async fn one_person_in_both_roles_is_returned_for_both() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let user_id = seed_user(&db).await;
        let mine = relationship(&db, user_id, user_id).await;

        let (coach, coachee) = find_coach_and_coachee(&db, mine).await?;

        assert_eq!(coach.id, user_id);
        assert_eq!(coachee.id, user_id);

        Ok::<(), Error>(())
    })
    .await
}

/// An unknown relationship is not found.
#[tokio::test]
async fn an_unknown_relationship_is_not_found() {
    within_time_limit(async {
        let db = database().await;

        assert!(find_coach_and_coachee(&db, Id::new_v4()).await.is_err());
    })
    .await
}
