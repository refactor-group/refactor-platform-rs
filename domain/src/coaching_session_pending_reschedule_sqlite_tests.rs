//! Which unannounced reschedules come due, who may claim them, and what completion and
//! failure leave behind, against a real SQL engine.

use chrono::{NaiveDate, NaiveDateTime, TimeDelta};
use entity::{coaching_sessions, Id};
use entity_api::coaching_session_pending_reschedule::{
    self, Claimed, Completed, Recorded, Released, Schedule, Timing, MAX_ATTEMPTS,
};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel, Set};

use crate::test_utils::sqlite::{database, seed_coaching_session, within_time_limit};

fn at(hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 12, 1)
        .and_then(|day| day.and_hms_opt(hour, minute, 0))
        .expect("valid test time")
}

fn now() -> NaiveDateTime {
    at(12, 0)
}

fn secs(n: i64) -> TimeDelta {
    TimeDelta::seconds(n)
}

fn timing() -> Timing {
    Timing {
        settle: secs(60),
        settle_max: secs(300),
        lease: secs(120),
    }
}

fn schedule(hour: u32, duration_minutes: i16) -> Schedule {
    Schedule {
        date: at(hour, 0),
        duration_minutes,
    }
}

fn a() -> Schedule {
    schedule(15, 60)
}

fn b() -> Schedule {
    schedule(16, 60)
}

fn c() -> Schedule {
    schedule(17, 45)
}

/// A session whose stored start is `date`.
async fn session_at(db: &DatabaseConnection, date: NaiveDateTime) -> Id {
    let (session_id, _) = seed_coaching_session(db).await;
    let mut session = coaching_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("the session reads")
        .expect("the session exists")
        .into_active_model();
    session.date = Set(date);
    session.update(db).await.expect("the session moves");
    session_id
}

/// A session starting well outside the cap, so only quiet and cap decide.
async fn later_session(db: &DatabaseConnection) -> Id {
    session_at(db, at(15, 0)).await
}

async fn record(
    db: &DatabaseConnection,
    session_id: Id,
    before: Schedule,
    after: Schedule,
    when: NaiveDateTime,
) -> Recorded {
    coaching_session_pending_reschedule::record_change(
        db,
        session_id,
        before,
        after,
        when,
        timing().lease,
    )
    .await
    .expect("the change records")
}

async fn claim(db: &DatabaseConnection, when: NaiveDateTime, claim_id: Id) -> Vec<Claimed> {
    coaching_session_pending_reschedule::claim_due(db, when, timing(), claim_id, 10)
        .await
        .expect("the claim runs")
}

async fn find(
    db: &DatabaseConnection,
    session_id: Id,
) -> Option<entity::coaching_session_pending_reschedules::Model> {
    coaching_session_pending_reschedule::find(db, session_id)
        .await
        .expect("the row reads")
}

#[tokio::test]
async fn first_change_records_the_pre_edit_schedule_as_announced() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        assert_eq!(
            record(&db, session_id, a(), b(), now()).await,
            Recorded::Pending
        );

        let row = find(&db, session_id).await.expect("a row is pending");
        assert_eq!(row.announced_date, a().date);
        assert_eq!(row.announced_duration_minutes, a().duration_minutes);
        assert_eq!(row.unannounced_since, now());
        assert_eq!(row.changed_at, now());
        assert_eq!(row.revision, 0);
        assert_eq!(row.attempts, 0);
        assert_eq!(row.claim_id, None);
        assert_eq!(row.claimed_at, None);
    })
    .await;
}

#[tokio::test]
async fn a_later_change_keeps_announced_and_bumps_the_revision() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        assert_eq!(
            record(&db, session_id, b(), c(), now() + secs(30)).await,
            Recorded::Pending
        );

        let row = find(&db, session_id).await.expect("a row is pending");
        assert_eq!(row.announced_date, a().date);
        assert_eq!(row.announced_duration_minutes, a().duration_minutes);
        assert_eq!(row.unannounced_since, now());
        assert_eq!(row.changed_at, now() + secs(30));
        assert_eq!(row.revision, 1);
    })
    .await;
}

#[tokio::test]
async fn undo_back_to_announced_deletes_an_unclaimed_row() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        assert_eq!(
            record(&db, session_id, b(), a(), now() + secs(10)).await,
            Recorded::Undone
        );
        assert_eq!(find(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn a_duration_only_difference_is_not_an_undo() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;
        let a_longer = Schedule {
            duration_minutes: 90,
            ..a()
        };

        record(&db, session_id, a(), b(), now()).await;
        assert_eq!(
            record(&db, session_id, b(), a_longer, now() + secs(10)).await,
            Recorded::Pending
        );
        assert!(find(&db, session_id).await.is_some());
    })
    .await;
}

#[tokio::test]
async fn undo_keeps_a_claimed_row_and_bumps_its_revision() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;
        let claim_id = Id::new_v4();

        record(&db, session_id, a(), b(), now()).await;
        assert_eq!(claim(&db, now() + secs(61), claim_id).await.len(), 1);
        assert_eq!(
            record(&db, session_id, b(), a(), now() + secs(70)).await,
            Recorded::Pending
        );

        let row = find(&db, session_id).await.expect("the claimed row stays");
        assert_eq!(row.claim_id, Some(claim_id));
        assert_eq!(row.revision, 1);
    })
    .await;
}

#[tokio::test]
async fn undo_deletes_a_row_whose_claim_lease_expired() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        assert_eq!(claim(&db, now() + secs(61), Id::new_v4()).await.len(), 1);
        let after_lease = now() + secs(61) + timing().lease + secs(1);
        assert_eq!(
            record(&db, session_id, b(), a(), after_lease).await,
            Recorded::Undone
        );
        assert_eq!(find(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn nothing_is_due_before_the_quiet_period() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        assert!(claim(&db, now() + secs(59), Id::new_v4()).await.is_empty());
        assert_eq!(
            find(&db, session_id).await.and_then(|row| row.claim_id),
            None
        );
    })
    .await;
}

#[tokio::test]
async fn a_change_is_due_once_the_quiet_period_passes() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;
        let claim_id = Id::new_v4();

        record(&db, session_id, a(), b(), now()).await;
        let claimed = claim(&db, now() + secs(60), claim_id).await;

        assert_eq!(
            claimed,
            vec![Claimed {
                coaching_session_id: session_id,
                claim_id,
                revision: 0,
                announced: a(),
            }]
        );
        let row = find(&db, session_id).await.expect("the row is claimed");
        assert_eq!(row.claim_id, Some(claim_id));
        assert_eq!(row.claimed_at, Some(now() + secs(60)));
    })
    .await;
}

#[tokio::test]
async fn each_change_restarts_the_quiet_period() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        record(&db, session_id, b(), c(), now() + secs(50)).await;

        assert!(claim(&db, now() + secs(100), Id::new_v4()).await.is_empty());
        assert_eq!(claim(&db, now() + secs(110), Id::new_v4()).await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_change_is_due_at_the_cap_despite_recent_changes() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        let mut previous = a();
        for (step, next) in [b(), c(), b(), c(), b(), c()].into_iter().enumerate() {
            let when = now() + secs(50 * step as i64);
            record(&db, session_id, previous, next, when).await;
            previous = next;
        }

        assert!(claim(&db, now() + secs(299), Id::new_v4()).await.is_empty());
        assert_eq!(claim(&db, now() + secs(300), Id::new_v4()).await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_session_starting_within_the_cap_is_due_without_the_quiet_period() {
    within_time_limit(async {
        let db = database().await;
        let soon = session_at(&db, now() + secs(240)).await;
        let later = session_at(&db, now() + secs(600)).await;

        record(&db, soon, a(), b(), now()).await;
        record(&db, later, a(), b(), now()).await;

        let claimed = claim(&db, now() + secs(1), Id::new_v4()).await;
        let ids: Vec<Id> = claimed.iter().map(|c| c.coaching_session_id).collect();
        assert_eq!(ids, vec![soon]);
    })
    .await;
}

#[tokio::test]
async fn a_claimed_row_is_not_claimed_again_until_its_lease_expires() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;
        let first = Id::new_v4();
        let second = Id::new_v4();

        record(&db, session_id, a(), b(), now()).await;
        assert_eq!(claim(&db, now() + secs(60), first).await.len(), 1);
        assert!(claim(&db, now() + secs(61), Id::new_v4()).await.is_empty());
        assert!(claim(
            &db,
            now() + secs(60) + timing().lease - secs(1),
            Id::new_v4()
        )
        .await
        .is_empty());

        let reclaimed = claim(&db, now() + secs(60) + timing().lease, second).await;
        assert_eq!(reclaimed.len(), 1);
        assert_eq!(reclaimed[0].claim_id, second);
    })
    .await;
}

#[tokio::test]
async fn a_claim_takes_at_most_the_limit_oldest_first() {
    within_time_limit(async {
        let db = database().await;
        let first = later_session(&db).await;
        let second = later_session(&db).await;
        let third = later_session(&db).await;

        record(&db, first, a(), b(), now()).await;
        record(&db, second, a(), b(), now() + secs(1)).await;
        record(&db, third, a(), b(), now() + secs(2)).await;

        let when = now() + secs(120);
        let taken =
            coaching_session_pending_reschedule::claim_due(&db, when, timing(), Id::new_v4(), 2)
                .await
                .expect("the claim runs");
        let ids: Vec<Id> = taken.iter().map(|c| c.coaching_session_id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&first) && ids.contains(&second));

        let rest = claim(&db, when, Id::new_v4()).await;
        assert_eq!(
            rest.iter()
                .map(|c| c.coaching_session_id)
                .collect::<Vec<_>>(),
            vec![third]
        );
    })
    .await;
}

#[tokio::test]
async fn completing_at_the_claimed_revision_settles() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        let claimed = claim(&db, now() + secs(60), Id::new_v4()).await;

        let completed = coaching_session_pending_reschedule::complete(&db, &claimed[0], b())
            .await
            .expect("completion runs");
        assert_eq!(completed, Completed::Settled);
        assert_eq!(find(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn a_change_during_the_send_stays_pending_from_what_was_sent() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        let claimed = claim(&db, now() + secs(60), Id::new_v4()).await;
        record(&db, session_id, b(), c(), now() + secs(65)).await;

        let completed = coaching_session_pending_reschedule::complete(&db, &claimed[0], b())
            .await
            .expect("completion runs");
        assert_eq!(completed, Completed::StillPending);

        let row = find(&db, session_id)
            .await
            .expect("the change stays pending");
        assert_eq!(row.announced_date, b().date);
        assert_eq!(row.announced_duration_minutes, b().duration_minutes);
        assert_eq!(row.claim_id, None);
        assert_eq!(row.claimed_at, None);
        assert_eq!(row.attempts, 0);

        let again = claim(&db, now() + secs(125), Id::new_v4()).await;
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].announced, b());
    })
    .await;
}

#[tokio::test]
async fn a_change_during_a_send_at_the_cap_restarts_the_cap_from_the_claim() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        let claimed = claim(&db, now() + secs(300), Id::new_v4()).await;
        record(&db, session_id, b(), c(), now() + secs(301)).await;
        coaching_session_pending_reschedule::complete(&db, &claimed[0], b())
            .await
            .expect("completion runs");

        let row = find(&db, session_id)
            .await
            .expect("the change stays pending");
        assert_eq!(row.unannounced_since, now() + secs(300));
        assert!(claim(&db, now() + secs(302), Id::new_v4()).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn an_undo_during_the_send_leaves_a_correction_pending() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        let claimed = claim(&db, now() + secs(60), Id::new_v4()).await;
        assert_eq!(
            record(&db, session_id, b(), a(), now() + secs(65)).await,
            Recorded::Pending
        );

        let completed = coaching_session_pending_reschedule::complete(&db, &claimed[0], b())
            .await
            .expect("completion runs");
        assert_eq!(completed, Completed::StillPending);
        let row = find(&db, session_id)
            .await
            .expect("a correction is pending");
        assert_eq!(row.announced_date, b().date);
    })
    .await;
}

#[tokio::test]
async fn a_completion_whose_claim_was_taken_over_changes_nothing() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;
        let second = Id::new_v4();

        record(&db, session_id, a(), b(), now()).await;
        let stale = claim(&db, now() + secs(60), Id::new_v4()).await;
        assert_eq!(
            claim(&db, now() + secs(60) + timing().lease, second)
                .await
                .len(),
            1
        );

        let completed = coaching_session_pending_reschedule::complete(&db, &stale[0], b())
            .await
            .expect("completion runs");
        assert_eq!(completed, Completed::Superseded);

        let row = find(&db, session_id)
            .await
            .expect("the newer claim holds the row");
        assert_eq!(row.claim_id, Some(second));
        assert_eq!(row.announced_date, a().date);

        let released = coaching_session_pending_reschedule::release(&db, &stale[0])
            .await
            .expect("release runs");
        assert_eq!(released, Released::Superseded);
        let row = find(&db, session_id)
            .await
            .expect("the newer claim holds the row");
        assert_eq!(row.claim_id, Some(second));
        assert_eq!(row.attempts, 0);
    })
    .await;
}

#[tokio::test]
async fn a_failed_send_releases_the_claim_and_counts_the_attempt() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        let claimed = claim(&db, now() + secs(60), Id::new_v4()).await;

        let released = coaching_session_pending_reschedule::release(&db, &claimed[0])
            .await
            .expect("release runs");
        assert_eq!(released, Released::Retrying);

        let row = find(&db, session_id)
            .await
            .expect("the change stays pending");
        assert_eq!(row.claim_id, None);
        assert_eq!(row.claimed_at, None);
        assert_eq!(row.attempts, 1);
        assert_eq!(row.announced_date, a().date);
        assert_eq!(claim(&db, now() + secs(60), Id::new_v4()).await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_failed_send_does_not_charge_a_change_made_during_it() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        let claimed = claim(&db, now() + secs(60), Id::new_v4()).await;
        record(&db, session_id, b(), c(), now() + secs(65)).await;
        coaching_session_pending_reschedule::release(&db, &claimed[0])
            .await
            .expect("release runs");

        let row = find(&db, session_id)
            .await
            .expect("the change stays pending");
        assert_eq!(row.claim_id, None);
        assert_eq!(row.attempts, 0);
    })
    .await;
}

#[tokio::test]
async fn the_last_allowed_failure_gives_up() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;
        assert_eq!(MAX_ATTEMPTS, 3);

        record(&db, session_id, a(), b(), now()).await;
        let mut outcomes = Vec::new();
        for _ in 0..MAX_ATTEMPTS {
            let claimed = claim(&db, now() + secs(60), Id::new_v4()).await;
            assert_eq!(claimed.len(), 1);
            outcomes.push(
                coaching_session_pending_reschedule::release(&db, &claimed[0])
                    .await
                    .expect("release runs"),
            );
        }

        assert_eq!(
            outcomes,
            vec![Released::Retrying, Released::Retrying, Released::GaveUp]
        );
        assert_eq!(find(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn a_new_change_restores_the_full_retry_budget() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        for _ in 0..MAX_ATTEMPTS - 1 {
            let claimed = claim(&db, now() + secs(60), Id::new_v4()).await;
            coaching_session_pending_reschedule::release(&db, &claimed[0])
                .await
                .expect("release runs");
        }
        record(&db, session_id, b(), c(), now() + secs(90)).await;

        let row = find(&db, session_id)
            .await
            .expect("the change stays pending");
        assert_eq!(row.attempts, 0);
    })
    .await;
}

async fn announce(db: &DatabaseConnection, session_id: Id, sent: Schedule, current: Schedule) {
    coaching_session_pending_reschedule::announce(db, session_id, sent, current, now() + secs(90))
        .await
        .expect("the announcement records");
}

#[tokio::test]
async fn announcing_the_current_schedule_drops_the_pending_change() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        announce(&db, session_id, b(), b()).await;

        assert_eq!(find(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn announcing_a_superseded_schedule_keeps_the_newer_change_pending_from_it() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        record(&db, session_id, b(), c(), now() + secs(30)).await;
        announce(&db, session_id, b(), c()).await;

        let row = find(&db, session_id)
            .await
            .expect("the newer change stays pending");
        assert_eq!(row.announced_date, b().date);
        assert_eq!(row.announced_duration_minutes, b().duration_minutes);
        assert_eq!(row.unannounced_since, now() + secs(90));
        assert_eq!(row.changed_at, now() + secs(30));
        assert_eq!(row.attempts, 0);
    })
    .await;
}

#[tokio::test]
async fn announcing_after_an_undo_left_nothing_pending_records_the_way_back() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        announce(&db, session_id, b(), a()).await;

        let row = find(&db, session_id)
            .await
            .expect("the way back is pending");
        assert_eq!(row.announced_date, b().date);
        let due = claim(&db, now() + secs(90) + secs(60), Id::new_v4()).await;
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].announced, b());
    })
    .await;
}

#[tokio::test]
async fn clear_removes_a_pending_change_and_is_idempotent() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        coaching_session_pending_reschedule::clear(&db, session_id)
            .await
            .expect("clear runs");
        assert_eq!(find(&db, session_id).await, None);
        coaching_session_pending_reschedule::clear(&db, session_id)
            .await
            .expect("a second clear runs");
    })
    .await;
}

#[tokio::test]
async fn deleting_the_session_drops_its_pending_change() {
    within_time_limit(async {
        let db = database().await;
        let session_id = later_session(&db).await;
        let other = later_session(&db).await;

        record(&db, session_id, a(), b(), now()).await;
        record(&db, other, a(), b(), now()).await;
        coaching_sessions::Entity::delete_by_id(session_id)
            .exec(&db)
            .await
            .expect("the session deletes");

        assert_eq!(find(&db, session_id).await, None);
        assert!(find(&db, other).await.is_some());
    })
    .await;
}
