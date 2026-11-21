//! What the pending reschedule sweep sends and leaves behind, against a real SQL engine.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeDelta, Timelike, Utc};
use mockito::{Matcher, Server, ServerGuard};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel, Set};
use serde_json::json;

use super::*;
use crate::test_utils::jobs::job_context_with;
use crate::test_utils::sqlite::fixture::{make_colleagues, seed_session_starting_in};
use crate::test_utils::sqlite::read_back::pending_invites;
use crate::test_utils::sqlite::{
    database, mirror_migration_defaults, organization_of, seed_coaching_session, seed_fixture,
    seed_membership, seed_organization, seed_user, within_time_limit, Caller, Fixture,
};
use crate::{coaching_relationships, coaching_session_pending_reschedules, coaching_sessions, Id};
use entity::coaching_session_pending_invites;

const TEMPLATE: &str = "session_rescheduled_tpl";

fn sending(server: &ServerGuard) -> Config {
    Config::from_args([
        "test",
        "--resend-api-key=test_api_key",
        &format!("--session-rescheduled-email-template-id={TEMPLATE}"),
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={}", server.url()),
    ])
}

async fn session(db: &DatabaseConnection, id: Id) -> coaching_sessions::Model {
    coaching_sessions::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the session reads")
        .expect("the session exists")
}

async fn reschedule(db: &DatabaseConnection, id: Id, start: NaiveDateTime) {
    let mut row = session(db, id).await.into_active_model();
    row.date = Set(start);
    row.update(db).await.expect("the session moves");
}

/// When the session's coachee was last told its start, if ever.
async fn coachee_notice(db: &DatabaseConnection, id: Id) -> Option<DateTime<FixedOffset>> {
    let relationship =
        coaching_relationships::Entity::find_by_id(session(db, id).await.coaching_relationship_id)
            .one(db)
            .await
            .expect("the relationship reads")
            .expect("the relationship exists");
    notices(db, id).await.get(&relationship.coachee_id).copied()
}

async fn pending(
    db: &DatabaseConnection,
    id: Id,
) -> Option<coaching_session_pending_reschedules::Model> {
    coaching_session_pending_reschedule::find(db, id)
        .await
        .expect("the pending row reads")
}

fn schedule(date: NaiveDateTime) -> Schedule {
    Schedule {
        date,
        duration_minutes: 60,
    }
}

/// A session a week out whose participant is not in its organization; returns its id
/// and start.
async fn upcoming_session_without_members(db: &DatabaseConnection) -> (Id, NaiveDateTime, Id) {
    mirror_migration_defaults(db).await;
    let (id, user_id) = seed_coaching_session(db).await;
    let start = (Utc::now() + TimeDelta::days(7))
        .naive_utc()
        .with_nanosecond(0)
        .expect("whole seconds");
    reschedule(db, id, start).await;
    (id, start, user_id)
}

/// A session a week out; returns its id and start.
async fn upcoming_session(db: &DatabaseConnection) -> (Id, NaiveDateTime) {
    let (id, start, user_id) = upcoming_session_without_members(db).await;
    seed_membership(db, user_id, organization_of(db, id).await).await;
    (id, start)
}

/// Moves a session an hour on, recorded as of `at`; returns its id and announced start.
async fn moved_session(db: &DatabaseConnection, at: NaiveDateTime) -> (Id, NaiveDateTime) {
    let (id, start) = upcoming_session(db).await;
    move_and_record(db, id, start, at).await;
    (id, start)
}

/// Moves a session an hour on from `start`, recorded as of `at`.
async fn move_and_record(db: &DatabaseConnection, id: Id, start: NaiveDateTime, at: NaiveDateTime) {
    let moved = start + TimeDelta::hours(1);
    reschedule(db, id, moved).await;
    coaching_session_pending_reschedule::record_change(
        db,
        id,
        schedule(start),
        schedule(moved),
        at,
        coaching_session_pending_reschedule::LEASE,
    )
    .await
    .expect("the change records");
}

/// Long enough ago that the default settle period has passed.
fn settled() -> NaiveDateTime {
    Utc::now().naive_utc() - TimeDelta::minutes(2)
}

#[tokio::test]
async fn a_due_change_is_emailed_and_settled() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let resend = server
            .mock("POST", "/emails")
            .with_status(200)
            .with_body(r#"{"id":"email_1"}"#)
            .expect_at_least(1)
            .create_async()
            .await;
        let db = database().await;
        let (id, _) = moved_session(&db, settled()).await;
        let before = coachee_notice(&db, id).await;
        let ctx = job_context_with(db, sending(&server));

        let outcome = Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        resend.assert_async().await;
        assert_eq!((outcome.processed, outcome.attempted), (1, 1));
        assert_eq!(pending(&ctx.db, id).await, None);
        assert!(coachee_notice(&ctx.db, id).await > before);
    })
    .await;
}

#[tokio::test]
async fn a_coachee_no_longer_in_the_organization_is_not_emailed() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let resend = server
            .mock("POST", "/emails")
            .with_status(200)
            .with_body(r#"{"id":"email_1"}"#)
            .expect(0)
            .create_async()
            .await;
        let db = database().await;
        let (id, start, _) = upcoming_session_without_members(&db).await;
        move_and_record(&db, id, start, settled()).await;
        let before = coachee_notice(&db, id).await;
        let ctx = job_context_with(db, sending(&server));

        let outcome = Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        resend.assert_async().await;
        assert_eq!((outcome.processed, outcome.attempted), (1, 1));
        assert_eq!(pending(&ctx.db, id).await, None);
        assert_eq!(coachee_notice(&ctx.db, id).await, before);
    })
    .await;
}

#[tokio::test]
async fn the_email_says_it_moved_from_the_last_announced_start() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let db = database().await;
        let (id, announced) = moved_session(&db, settled()).await;
        let resend = server
            .mock("POST", "/emails")
            .match_body(Matcher::PartialJson(json!({
                "template": {
                    "id": TEMPLATE,
                    "variables": {
                        "previous_session_when":
                            announced.format("%A, %B %-d, %Y at %-I:%M %p").to_string()
                    }
                }
            })))
            .with_status(200)
            .with_body(r#"{"id":"email_1"}"#)
            .expect_at_least(1)
            .create_async()
            .await;
        let ctx = job_context_with(db, sending(&server));

        Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        resend.assert_async().await;
        assert_eq!(pending(&ctx.db, id).await, None);
    })
    .await;
}

#[tokio::test]
async fn a_failed_send_is_retried_then_given_up() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let _resend = server
            .mock("POST", "/emails")
            .with_status(500)
            .with_body("down")
            .create_async()
            .await;
        let db = database().await;
        let (id, _) = moved_session(&db, settled()).await;
        let before = coachee_notice(&db, id).await;
        let ctx = job_context_with(db, sending(&server));
        let sweep = Sweep::from_config(&ctx.config);

        let outcome = sweep.run(&ctx).await.expect("the tick runs");

        assert_eq!((outcome.processed, outcome.attempted), (0, 1));
        let row = pending(&ctx.db, id).await.expect("the change is retried");
        assert_eq!(row.attempts, 1);
        assert_eq!(row.claim_id, None);
        assert_eq!(coachee_notice(&ctx.db, id).await, before);

        for _ in 1..coaching_session_pending_reschedule::MAX_ATTEMPTS {
            sweep.run(&ctx).await.expect("the tick runs");
        }

        assert_eq!(pending(&ctx.db, id).await, None);
    })
    .await;
}

#[tokio::test]
async fn nothing_to_tell_settles_without_an_email() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let resend = server
            .mock("POST", "/emails")
            .with_status(200)
            .expect(0)
            .create_async()
            .await;
        let db = database().await;
        let (id, announced) = moved_session(&db, settled()).await;
        reschedule(&db, id, announced).await;
        let ctx = job_context_with(db, sending(&server));

        Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        resend.assert_async().await;
        assert_eq!(pending(&ctx.db, id).await, None);
    })
    .await;
}

#[tokio::test]
async fn drain_mode_drops_due_changes_without_sending() {
    within_time_limit(async {
        let db = database().await;
        let (id, _) = moved_session(&db, settled()).await;
        let before = coachee_notice(&db, id).await;
        let ctx = job_context_with(db, Config::from_args(["test"]));
        let sweep = Sweep::from_config(&ctx.config);
        assert_eq!(sweep.mode, Mode::Drain);

        sweep.run(&ctx).await.expect("the tick runs");

        assert_eq!(pending(&ctx.db, id).await, None);
        assert_eq!(coachee_notice(&ctx.db, id).await, before);
    })
    .await;
}

#[tokio::test]
async fn a_change_that_is_not_due_is_left_alone() {
    within_time_limit(async {
        let server = Server::new_async().await;
        let db = database().await;
        let (id, _) = moved_session(&db, Utc::now().naive_utc()).await;
        let before = pending(&db, id).await.expect("the change is pending");
        let ctx = job_context_with(db, sending(&server));

        let outcome = Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        assert_eq!((outcome.processed, outcome.attempted), (0, 0));
        let after = pending(&ctx.db, id).await.expect("the change is pending");
        assert_eq!(after.claim_id, None);
        assert_eq!(after, before);
    })
    .await;
}

/// Which participants of [`moved_session_between_two`] hold a role in its organization.
#[derive(Clone, Copy)]
enum Members {
    Both,
    CoacheeOnly,
    CoachOnly,
}

impl Members {
    fn coach(self) -> bool {
        matches!(self, Members::Both | Members::CoachOnly)
    }

    fn coachee(self) -> bool {
        matches!(self, Members::Both | Members::CoacheeOnly)
    }
}

/// A session a week out between a separate coach and coachee, each with a notice row an
/// hour old, moved and recorded as settled; `members` say who holds a role.
/// Returns its id, coach and coachee.
async fn moved_session_between_two(db: &DatabaseConnection, members: Members) -> (Id, Id, Id) {
    mirror_migration_defaults(db).await;
    let (coach_id, coachee_id) = (seed_user(db).await, seed_user(db).await);
    let organization_id = seed_organization(db).await;
    if members.coachee() {
        seed_membership(db, coachee_id, organization_id).await;
    }
    if members.coach() {
        seed_membership(db, coach_id, organization_id).await;
    }
    let now = Utc::now();
    let relationship_id = Id::new_v4();
    coaching_relationships::ActiveModel {
        id: Set(relationship_id),
        organization_id: Set(organization_id),
        coach_id: Set(coach_id),
        coachee_id: Set(coachee_id),
        kind: Set(crate::coaching_relationship_kind::Kind::Coaching),
        slug: Set(format!("relationship-{relationship_id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the relationship is seeded");
    let start = (now + TimeDelta::days(7))
        .naive_utc()
        .with_nanosecond(0)
        .expect("whole seconds");
    let id = coaching_sessions::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_relationship_id: Set(relationship_id),
        date: Set(start),
        duration_minutes: Set(60),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        scheduled_by_user_id: Set(coach_id),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("the session is seeded")
    .id;
    entity_api::coaching_session_notice::stamp(
        db,
        id,
        &[coach_id, coachee_id],
        (now - TimeDelta::hours(1)).into(),
    )
    .await
    .expect("the notices stamp");
    move_and_record(db, id, start, settled()).await;
    (id, coach_id, coachee_id)
}

async fn notices(db: &DatabaseConnection, id: Id) -> BTreeMap<Id, DateTime<FixedOffset>> {
    entity_api::coaching_session_notice::find_by_coaching_session(db, id)
        .await
        .expect("the notices read")
        .into_iter()
        .map(|row| (row.user_id, row.notice_given_at))
        .collect()
}

async fn resend_answering(server: &mut ServerGuard, status: usize) -> mockito::Mock {
    server
        .mock("POST", "/emails")
        .with_status(status)
        .with_body(r#"{"id":"email_1"}"#)
        .create_async()
        .await
}

#[tokio::test]
async fn a_change_told_to_both_restamps_each_participants_notice() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let _resend = resend_answering(&mut server, 200).await;
        let db = database().await;
        let (id, coach_id, coachee_id) = moved_session_between_two(&db, Members::Both).await;
        let before = notices(&db, id).await;
        let ctx = job_context_with(db, sending(&server));

        Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        let after = notices(&ctx.db, id).await;
        assert!(after[&coach_id] > before[&coach_id]);
        assert!(after[&coachee_id] > before[&coachee_id]);
    })
    .await;
}

#[tokio::test]
async fn a_coach_no_longer_in_the_organization_keeps_their_notice() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let _resend = resend_answering(&mut server, 200).await;
        let db = database().await;
        let (id, coach_id, coachee_id) = moved_session_between_two(&db, Members::CoacheeOnly).await;
        let before = notices(&db, id).await;
        let ctx = job_context_with(db, sending(&server));

        Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        let after = notices(&ctx.db, id).await;
        assert!(after[&coachee_id] > before[&coachee_id]);
        assert_eq!(after[&coach_id], before[&coach_id]);
    })
    .await;
}

#[tokio::test]
async fn a_coachee_no_longer_in_the_organization_leaves_only_the_coach_told() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let _resend = resend_answering(&mut server, 200).await;
        let db = database().await;
        let (id, coach_id, coachee_id) = moved_session_between_two(&db, Members::CoachOnly).await;
        let before = notices(&db, id).await;
        let ctx = job_context_with(db, sending(&server));

        Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        let after = notices(&ctx.db, id).await;
        assert!(after[&coach_id] > before[&coach_id]);
        assert_eq!(after[&coachee_id], before[&coachee_id]);
    })
    .await;
}

#[tokio::test]
async fn a_change_told_to_nobody_restamps_no_notice() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let _resend = resend_answering(&mut server, 500).await;
        let db = database().await;
        let (id, _, _) = moved_session_between_two(&db, Members::Both).await;
        let before = notices(&db, id).await;
        let ctx = job_context_with(db, sending(&server));

        Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        assert_eq!(notices(&ctx.db, id).await, before);
    })
    .await;
}

/// Sweeps a colleague session the coachee-seat colleague moved, with Resend failing for the
/// coach seat's copy (`coach_fails`) or the coachee seat's; returns whether the change is
/// still pending afterwards.
async fn sweep_colleague_change_failing_for(coach_fails: bool) -> bool {
    let mut server = Server::new_async().await;
    let db = database().await;
    let (id, coach_id, coachee_id) = moved_session_between_two(&db, Members::Both).await;
    make_colleagues(&db, session(&db, id).await.coaching_relationship_id).await;
    coaching_sessions::ActiveModel {
        id: Set(id),
        scheduled_by_user_id: Set(coachee_id),
        ..Default::default()
    }
    .update(&db)
    .await
    .expect("the coachee-seat colleague moved it");
    let (failing, told) = match coach_fails {
        true => (coach_id, coachee_id),
        false => (coachee_id, coach_id),
    };
    let _failing = server
        .mock("POST", "/emails")
        .match_body(Matcher::Regex(failing.to_string()))
        .with_status(500)
        .with_body("down")
        .create_async()
        .await;
    let _told = server
        .mock("POST", "/emails")
        .match_body(Matcher::Regex(told.to_string()))
        .with_status(200)
        .with_body(r#"{"id":"email_1"}"#)
        .create_async()
        .await;
    let ctx = job_context_with(db, sending(&server));

    Sweep::from_config(&ctx.config)
        .run(&ctx)
        .await
        .expect("the tick runs");

    pending(&ctx.db, id).await.is_some()
}

/// A colleague's change is retried until the other colleague is told.
#[tokio::test]
async fn a_colleague_change_the_other_colleague_missed_is_retried() {
    within_time_limit(async {
        assert!(sweep_colleague_change_failing_for(true).await);
    })
    .await;
}

/// The colleague who moved a session already knows, so missing only their copy settles it.
#[tokio::test]
async fn a_colleague_change_that_missed_only_the_scheduler_settles() {
    within_time_limit(async {
        assert!(!sweep_colleague_change_failing_for(false).await);
    })
    .await;
}

/// Sends series invites, and nothing else, to Resend at `server`; composes session invites.
fn inviting(server: &ServerGuard) -> Config {
    Config::from_args([
        "test",
        "--resend-api-key=test_api_key",
        "--recurring-sessions-scheduled-email-template-id=series_scheduled_tpl",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={}", server.url()),
    ])
}

/// Each outbox row's kind and addressee.
async fn outboxed(db: &DatabaseConnection) -> Vec<(String, Id)> {
    crate::test_utils::email_outbox::rows(db)
        .await
        .into_iter()
        .map(|row| (row.kind, row.recipient_user_id))
        .collect()
}

/// The seeded fixture with `count` more sessions of R1 owed to COACH and, when `both`,
/// COACHEE.
async fn owing(db: &DatabaseConnection, count: i64, both: bool) -> Fixture {
    mirror_migration_defaults(db).await;
    let fixture = seed_fixture(db).await;
    let mut sessions = BTreeSet::new();
    for day in 0..count {
        let id = seed_session_starting_in(db, fixture.r1, TimeDelta::days(10 + day)).await;
        sessions.insert(Target::Session(id));
    }
    let users: Vec<Id> = [Caller::Coach, Caller::Coachee]
        .into_iter()
        .take(if both { 2 } else { 1 })
        .map(|caller| fixture.user(caller))
        .collect();
    coaching_session_pending_invite::queue(db, &sessions, &users)
        .await
        .expect("the invites are owed");
    fixture
}

/// Each owed invite's failed attempts and whether a claim holds it.
async fn owed_rows(db: &DatabaseConnection) -> Vec<(i16, bool)> {
    let mut rows: Vec<(i16, bool)> = coaching_session_pending_invites::Entity::find()
        .all(db)
        .await
        .expect("the owed invites read")
        .into_iter()
        .map(|row| (row.attempts, row.claim_id.is_some()))
        .collect();
    rows.sort();
    rows
}

#[tokio::test]
async fn an_invite_owed_again_while_a_send_holds_it_stays_owed() {
    within_time_limit(async {
        let db = database().await;
        mirror_migration_defaults(&db).await;
        let fixture = seed_fixture(&db).await;
        let coach = fixture.user(Caller::Coach);
        let s1 = BTreeSet::from([Target::Session(fixture.s1)]);
        let claim = || {
            coaching_session_pending_invite::claim(&db, Utc::now().naive_utc(), Id::new_v4(), 10)
        };

        coaching_session_pending_invite::queue(&db, &s1, &[coach])
            .await
            .expect("the invite is owed");
        let in_flight = claim().await.expect("the sweep claims");
        coaching_session_pending_invite::queue(&db, &s1, &[coach])
            .await
            .expect("the invite is owed again");
        for row in &in_flight {
            coaching_session_pending_invite::complete(&db, row)
                .await
                .expect("the earlier send completes");
        }

        assert_eq!(
            pending_invites(&db).await,
            BTreeSet::from([(fixture.s1, coach)])
        );
        assert_eq!(claim().await.expect("the sweep claims").len(), 1);
    })
    .await;
}

#[tokio::test]
async fn an_owed_session_invite_is_written_and_settled_together() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let resend = server
            .mock("POST", "/emails")
            .expect(0)
            .create_async()
            .await;
        let db = database().await;
        let fixture = owing(&db, 1, true).await;
        let [(target, _), ..] = pending_invites(&db).await.into_iter().collect::<Vec<_>>()[..]
        else {
            panic!("an invite is owed");
        };
        let before = session(&db, target).await.ical_sequence;
        let ctx = job_context_with(db, inviting(&server));

        Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        let mut written = outboxed(&ctx.db).await;
        written.sort();
        let mut expected = [Caller::Coach, Caller::Coachee]
            .map(|caller| ("session_scheduled".to_string(), fixture.user(caller)))
            .to_vec();
        expected.sort();
        assert_eq!(written, expected);
        assert_eq!(owed_rows(&ctx.db).await, Vec::new());
        assert_eq!(
            session(&ctx.db, target).await.ical_sequence,
            before + 2,
            "each copy goes out under its own SEQUENCE"
        );
        resend.assert_async().await;
    })
    .await;
}

#[tokio::test]
async fn a_stale_claim_on_an_owed_session_invite_writes_nothing() {
    within_time_limit(async {
        let db = database().await;
        owing(&db, 1, false).await;
        let claimed = coaching_session_pending_invite::claim(
            &db,
            Utc::now().naive_utc(),
            Id::new_v4(),
            INVITE_BATCH,
        )
        .await
        .expect("the sweep claims");
        let [held] = <[_; 1]>::try_from(claimed).expect("one invite is owed");
        let Target::Session(id) = held.target else {
            panic!("a session's invite is owed");
        };
        let before = session(&db, id).await.ical_sequence;
        let stale = coaching_session_pending_invite::Claimed {
            claim_id: Id::new_v4(),
            ..held
        };

        crate::coaching_session::enqueue_owed_invite(
            &db,
            &inviting(&Server::new_async().await),
            id,
            &stale,
        )
        .await
        .expect("a stale claim is not an error");

        assert_eq!(outboxed(&db).await, Vec::new());
        assert_eq!(session(&db, id).await.ical_sequence, before);
        assert_eq!(
            owed_rows(&db).await,
            vec![(0, true)],
            "the holder still owes it"
        );
    })
    .await;
}

/// With no frontend to link to no invite composes, so the owed row is settled unsent rather
/// than retried.
#[tokio::test]
async fn an_owed_session_invite_that_cannot_compose_is_settled_unsent() {
    within_time_limit(async {
        let db = database().await;
        owing(&db, 1, false).await;
        let [(target, _), ..] = pending_invites(&db).await.into_iter().collect::<Vec<_>>()[..]
        else {
            panic!("an invite is owed");
        };
        let before = session(&db, target).await.ical_sequence;
        let ctx = job_context_with(db, Config::from_args(["test"]));

        Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        assert_eq!(owed_rows(&ctx.db).await, Vec::new());
        assert_eq!(outboxed(&ctx.db).await, Vec::new());
        assert_eq!(session(&ctx.db, target).await.ical_sequence, before);
    })
    .await;
}

#[tokio::test]
async fn many_owed_invites_drain_over_several_ticks() {
    within_time_limit(async {
        let server = Server::new_async().await;
        let db = database().await;
        owing(&db, 6, true).await;
        let ctx = job_context_with(db, inviting(&server));
        let sweep = Sweep::from_config(&ctx.config);

        sweep.run(&ctx).await.expect("the tick runs");
        assert_eq!(
            owed_rows(&ctx.db).await.len(),
            2,
            "ten written in the first tick"
        );
        assert_eq!(outboxed(&ctx.db).await.len(), 10);
        sweep.run(&ctx).await.expect("the tick runs");
        assert_eq!(owed_rows(&ctx.db).await, Vec::new());
        assert_eq!(outboxed(&ctx.db).await.len(), 12);
    })
    .await;
}

#[tokio::test]
async fn a_failed_series_invite_hands_the_rest_of_the_tick_back_uncharged() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let resend = server
            .mock("POST", "/emails")
            .with_status(429)
            .with_header("retry-after", "3")
            .expect(1)
            .create_async()
            .await;
        let db = database().await;
        mirror_migration_defaults(&db).await;
        let fixture = seed_fixture(&db).await;
        coaching_session_pending_invite::queue(
            &db,
            &BTreeSet::from([Target::Series(fixture.sr1)]),
            &[fixture.user(Caller::Coach), fixture.user(Caller::Coachee)],
        )
        .await
        .expect("the invites are owed");
        let ctx = job_context_with(db, inviting(&server));

        Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        resend.assert_async().await;
        assert_eq!(
            owed_rows(&ctx.db).await,
            vec![(0, false), (1, false)],
            "one attempt charged, none held"
        );
    })
    .await;
}
