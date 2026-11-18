//! What a confirmed switch writes when the relationship changed after it was first read.

use sea_orm::{ActiveModelTrait, IntoActiveModel, Set};

use super::*;
use crate::error::EntityErrorKind;
use crate::test_utils::sqlite::fixture::{make_colleagues, seed_session_starting_in};
use crate::test_utils::sqlite::read_back::{
    pending_invites, relationship_row, reminders_sent, session_schedule,
};
use crate::test_utils::sqlite::{
    database, mirror_migration_defaults, seed_fixture, within_time_limit, Caller, Fixture,
};

/// The seeded fixture, and R1 as it stood before the test changed anything.
async fn seeded() -> (DatabaseConnection, Fixture, Model) {
    let db = database().await;
    mirror_migration_defaults(&db).await;
    let fixture = seed_fixture(&db).await;
    let r1 = relationship_row(&db, fixture.r1).await.expect("R1 exists");
    (db, fixture, r1)
}

fn confirmed(kind: Kind, coach_id: Option<Id>) -> Request {
    Request {
        kind,
        coach_id,
        confirmed: true,
    }
}

#[tokio::test]
async fn a_switch_already_made_under_the_lock_writes_nothing() {
    within_time_limit(async {
        let (db, fixture, stale) = seeded().await;
        make_colleagues(&db, fixture.r1).await;
        let current = relationship_row(&db, fixture.r1).await;

        let applied = apply(
            &db,
            &Config::from_args(["test"]),
            &stale,
            confirmed(Kind::Colleague, None),
        )
        .await
        .expect("the switch applies");

        assert_eq!(applied, None);
        assert_eq!(relationship_row(&db, fixture.r1).await, current);
        assert_eq!(pending_invites(&db).await, BTreeSet::new());
    })
    .await;
}

#[tokio::test]
async fn a_switch_starts_from_the_relationship_as_it_stands_under_the_lock() {
    within_time_limit(async {
        let (db, fixture, stale) = seeded().await;
        let (coach, coachee) = (fixture.user(Caller::Coach), fixture.user(Caller::Coachee));
        make_colleagues(&db, fixture.r1).await;

        let (before, after) = apply(
            &db,
            &Config::from_args(["test"]),
            &stale,
            confirmed(Kind::Coaching, Some(coachee)),
        )
        .await
        .expect("the switch applies")
        .expect("the switch changes the relationship");

        assert_eq!(before.kind, Kind::Colleague, "read under the lock");
        assert_eq!(
            (after.kind, after.coach_id, after.coachee_id),
            (Kind::Coaching, coachee, coach)
        );
    })
    .await;
}

#[tokio::test]
async fn a_switch_suppresses_the_reminders_of_whom_it_newly_reminds_under_the_lock() {
    within_time_limit(async {
        let (db, fixture, _) = seeded().await;
        let coach = fixture.user(Caller::Coach);
        let imminent = seed_session_starting_in(&db, fixture.r1, TimeDelta::hours(2)).await;
        let (start, _) = session_schedule(&db, imminent)
            .await
            .expect("the session reads");
        make_colleagues(&db, fixture.r1).await;
        let stale = relationship_row(&db, fixture.r1).await.expect("R1 exists");
        // Switched back to coaching before the lock: the coach is no longer reminded.
        let mut coaching = stale.clone().into_active_model();
        coaching.kind = Set(Kind::Coaching);
        coaching.update(&db).await.expect("R1 is coaching again");

        apply(
            &db,
            &Config::from_args(["test"]),
            &stale,
            confirmed(Kind::Colleague, None),
        )
        .await
        .expect("the switch applies")
        .expect("the switch changes the relationship");

        assert_eq!(
            reminders_sent(&db, imminent).await,
            BTreeMap::from([(coach, start)]),
            "the coach is newly reminded as R1 stands under the lock, though not as first read"
        );
    })
    .await;
}

#[tokio::test]
async fn a_relationship_gone_under_the_lock_is_not_found() {
    within_time_limit(async {
        let (db, _, stale) = seeded().await;
        let gone = Model {
            id: Id::new_v4(),
            ..stale
        };

        let applied = apply(
            &db,
            &Config::from_args(["test"]),
            &gone,
            confirmed(Kind::Colleague, None),
        )
        .await;

        assert!(
            applied.as_ref().is_err_and(|e| e.error_kind
                == DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::NotFound))),
            "{applied:?}"
        );
        assert_eq!(pending_invites(&db).await, BTreeSet::new());
    })
    .await;
}
