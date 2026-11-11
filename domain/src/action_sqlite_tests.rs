//! A user's actions read against a real SQL engine: `coached_by` keeps only the
//! relationships that coach has with the user as coachee.

use std::collections::BTreeSet;

use chrono::Utc;
use entity::{actions, actions_users, coaching_relationships, coaching_sessions, status::Status};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};

use super::*;
use crate::test_utils::sqlite::{database, seed_organization, seed_user, within_time_limit};

/// COACH, COACHEE, OTHER_COACH and MENTEE across two organizations, with actions A1 to A6.
struct World {
    db: DatabaseConnection,
    coach: Id,
    coachee: Id,
    other_coach: Id,
    mentee: Id,
    r1: Id,
    r2: Id,
    a: [Id; 6],
}

/// Inserts a relationship and one session in it; returns both ids.
async fn relationship(
    db: &DatabaseConnection,
    organization_id: Id,
    coach_id: Id,
    coachee_id: Id,
) -> (Id, Id) {
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
    .expect("the relationship is seeded");

    let session_id = Id::new_v4();
    coaching_sessions::ActiveModel {
        id: Set(session_id),
        coaching_relationship_id: Set(id),
        coaching_session_series_id: Set(None),
        ical_sequence: Set(0),
        ical_recurrence_id: Set(None),
        collab_document_name: Set(None),
        date: Set(now.naive_utc()),
        duration_minutes: Set(60),
        title: Set(None),
        meeting_url: Set(None),
        provider: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        hydrated_at: Set(None),
        notice_given_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the session is seeded");

    (id, session_id)
}

/// Inserts an action in `session_id`, assigned to `assignee` when given.
async fn action(db: &DatabaseConnection, session_id: Id, owner_id: Id, assignee: Option<Id>) -> Id {
    let id = Id::new_v4();
    let now = Utc::now();
    actions::ActiveModel {
        id: Set(id),
        coaching_session_id: Set(session_id),
        goal_id: Set(None),
        user_id: Set(owner_id),
        body: Set(Some(format!("Action {id}"))),
        due_by: Set(None),
        status: Set(Status::InProgress),
        status_changed_at: Set(now.into()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the action is seeded");

    if let Some(user_id) = assignee {
        actions_users::ActiveModel {
            id: Set(Id::new_v4()),
            action_id: Set(id),
            user_id: Set(user_id),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(db)
        .await
        .expect("the assignment is seeded");
    }
    id
}

async fn world() -> World {
    let db = database().await;
    let (coach, coachee, other_coach, mentee) = (
        seed_user(&db).await,
        seed_user(&db).await,
        seed_user(&db).await,
        seed_user(&db).await,
    );
    let (o1, o2) = (seed_organization(&db).await, seed_organization(&db).await);

    let (r1, s1) = relationship(&db, o1, coach, coachee).await;
    let (r2, s2) = relationship(&db, o1, other_coach, coachee).await;
    let (_, s4) = relationship(&db, o1, coachee, mentee).await;
    let (_, s5) = relationship(&db, o2, coach, coachee).await;

    let a = [
        action(&db, s1, coach, Some(coachee)).await,
        action(&db, s1, coach, None).await,
        action(&db, s2, other_coach, Some(coachee)).await,
        action(&db, s2, other_coach, None).await,
        action(&db, s4, coachee, Some(mentee)).await,
        action(&db, s5, coach, Some(coachee)).await,
    ];

    World {
        db,
        coach,
        coachee,
        other_coach,
        mentee,
        r1,
        r2,
        a,
    }
}

impl World {
    /// The ids `find_by_user` returns for the coachee under `params`.
    async fn ids(&self, params: FindByUserParams) -> Result<BTreeSet<Id>, Error> {
        Ok(find_by_user(&self.db, self.coachee, params)
            .await?
            .into_iter()
            .map(|a| a.action.id)
            .collect())
    }

    /// Actions A`n` for each `n` in `numbers`.
    fn actions(&self, numbers: &[usize]) -> BTreeSet<Id> {
        numbers.iter().map(|n| self.a[n - 1]).collect()
    }
}

fn params(scope: Scope, coached_by: Option<Id>) -> FindByUserParams {
    FindByUserParams {
        scope,
        coached_by,
        ..Default::default()
    }
}

/// ACT-22
#[tokio::test]
async fn a_coach_sees_only_their_relationships_with_the_user() -> Result<(), Error> {
    within_time_limit(async {
        let w = world().await;
        let ids = w.ids(params(Scope::Sessions, Some(w.coach))).await?;
        assert_eq!(ids, w.actions(&[1, 2, 6]));
        Ok::<(), Error>(())
    })
    .await
}

/// ACT-23
#[tokio::test]
async fn a_coach_sees_only_assignments_from_their_relationships() -> Result<(), Error> {
    within_time_limit(async {
        let w = world().await;
        let ids = w.ids(params(Scope::Assigned, Some(w.coach))).await?;
        assert_eq!(ids, w.actions(&[1, 6]));
        Ok::<(), Error>(())
    })
    .await
}

/// ACT-24
#[tokio::test]
async fn another_coach_sees_only_their_own_relationship() -> Result<(), Error> {
    within_time_limit(async {
        let w = world().await;
        let ids = w.ids(params(Scope::Sessions, Some(w.other_coach))).await?;
        assert_eq!(ids, w.actions(&[3, 4]));
        Ok::<(), Error>(())
    })
    .await
}

/// ACT-24b
#[tokio::test]
async fn a_coach_naming_another_coachs_relationship_sees_nothing() -> Result<(), Error> {
    within_time_limit(async {
        let w = world().await;
        let ids = w
            .ids(FindByUserParams {
                coaching_relationship_id: Some(w.r2),
                ..params(Scope::Sessions, Some(w.coach))
            })
            .await?;
        assert_eq!(ids, BTreeSet::new());
        Ok::<(), Error>(())
    })
    .await
}

/// ACT-24c
#[tokio::test]
async fn a_coach_naming_their_relationship_sees_its_assignments() -> Result<(), Error> {
    within_time_limit(async {
        let w = world().await;
        let ids = w
            .ids(FindByUserParams {
                coaching_relationship_id: Some(w.r1),
                ..params(Scope::Assigned, Some(w.coach))
            })
            .await?;
        assert_eq!(ids, w.actions(&[1]));
        Ok::<(), Error>(())
    })
    .await
}

/// ACT-24d
#[tokio::test]
async fn the_users_own_coachee_sees_nothing_as_their_coach() -> Result<(), Error> {
    within_time_limit(async {
        let w = world().await;
        let ids = w.ids(params(Scope::Sessions, Some(w.mentee))).await?;
        assert_eq!(ids, BTreeSet::new());
        Ok::<(), Error>(())
    })
    .await
}

/// ACT-24e: the user's assignment in a relationship the coach has with someone else.
#[tokio::test]
async fn a_coach_does_not_see_assignments_from_their_other_coachees() -> Result<(), Error> {
    within_time_limit(async {
        let w = world().await;
        let organization_id = seed_organization(&w.db).await;
        let (_, s6) = relationship(&w.db, organization_id, w.coach, w.mentee).await;
        action(&w.db, s6, w.coach, Some(w.coachee)).await;

        let ids = w.ids(params(Scope::Assigned, Some(w.coach))).await?;
        assert_eq!(ids, w.actions(&[1, 6]));
        Ok::<(), Error>(())
    })
    .await
}

/// ACT-18
#[tokio::test]
async fn the_user_sees_every_relationship_they_are_in() -> Result<(), Error> {
    within_time_limit(async {
        let w = world().await;
        let ids = w.ids(params(Scope::Sessions, None)).await?;
        assert_eq!(ids, w.actions(&[1, 2, 3, 4, 5, 6]));
        Ok::<(), Error>(())
    })
    .await
}

/// ACT-19
#[tokio::test]
async fn the_user_sees_every_assignment_of_theirs() -> Result<(), Error> {
    within_time_limit(async {
        let w = world().await;
        let ids = w.ids(params(Scope::Assigned, None)).await?;
        assert_eq!(ids, w.actions(&[1, 3, 6]));
        Ok::<(), Error>(())
    })
    .await
}
