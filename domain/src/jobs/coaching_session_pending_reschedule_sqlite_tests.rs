//! What the pending reschedule sweep writes and leaves behind, against a real SQL engine.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeDelta, Timelike, Utc};
use entity_api::coaching_session_pending_reschedule::Schedule;
use mockito::{Server, ServerGuard};
use sea_orm::{
    ActiveModelTrait, ConnectionTrait, DatabaseConnection, EntityTrait, IntoActiveModel, Set,
};

use super::*;
use crate::email_outbox::OutgoingEmail;
use crate::test_utils::email_outbox::{calendar_part, emails};
use crate::test_utils::jobs::job_context_with;
use crate::test_utils::sqlite::fixture::{move_series_into_the_past, seed_session_starting_in};
use crate::test_utils::sqlite::read_back::pending_invites;
use crate::test_utils::sqlite::{
    database, mirror_migration_defaults, seed_fixture, seed_membership, seed_organization,
    seed_user, within_time_limit, Caller, Fixture,
};
use crate::{coaching_relationships, coaching_session_pending_reschedules, coaching_sessions, Id};
use entity::{coaching_session_pending_invites, coaching_session_series};

/// The `SEQUENCE` the seeded session's last edit left it at.
const SEQUENCE: i32 = 3;

/// Enough to compose an email, and no provider.
fn composing() -> Config {
    Config::from_args(["test", "--frontend-base-url=https://app.example.com"])
}

async fn session(db: &DatabaseConnection, id: Id) -> coaching_sessions::Model {
    coaching_sessions::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the session reads")
        .expect("the session exists")
}

/// Moves session `id` to `to` without recording it.
async fn reschedule(db: &DatabaseConnection, id: Id, to: Schedule) {
    let mut row = session(db, id).await.into_active_model();
    row.date = Set(to.date);
    row.duration_minutes = Set(to.duration_minutes);
    row.update(db).await.expect("the session changes");
}

/// Moves session `id` to `to` and records the change from `from`, as of `at`.
async fn change(db: &DatabaseConnection, id: Id, from: Schedule, to: Schedule, at: NaiveDateTime) {
    reschedule(db, id, to).await;
    coaching_session_pending_reschedule::record_change(
        db,
        id,
        from,
        to,
        at,
        coaching_session_pending_reschedule::LEASE,
    )
    .await
    .expect("the change records");
}

async fn pending(
    db: &DatabaseConnection,
    id: Id,
) -> Option<coaching_session_pending_reschedules::Model> {
    coaching_session_pending_reschedule::find(db, id)
        .await
        .expect("the pending row reads")
}

/// Long enough ago that the default settle period has passed.
fn settled() -> NaiveDateTime {
    Utc::now().naive_utc() - TimeDelta::minutes(2)
}

/// Which participants of [`changed_session`] hold a role in its organization.
#[derive(Clone, Copy)]
enum Members {
    Both,
    CoacheeOnly,
    Neither,
}

impl Members {
    fn coach(self) -> bool {
        matches!(self, Members::Both)
    }

    fn coachee(self) -> bool {
        matches!(self, Members::Both | Members::CoacheeOnly)
    }
}

/// A session a week out between a separate coach and coachee, each with a notice row an
/// hour old, at [`SEQUENCE`]; `members` say who holds a role.
struct Seeded {
    id: Id,
    relationship_id: Id,
    coach_id: Id,
    coachee_id: Id,
    announced: Schedule,
}

impl Seeded {
    async fn new(db: &DatabaseConnection, members: Members) -> Self {
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
            ical_sequence: Set(SEQUENCE),
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
        Self {
            id,
            relationship_id,
            coach_id,
            coachee_id,
            announced: Schedule {
                date: start,
                duration_minutes: 60,
            },
        }
    }

    /// Changes the session from what was announced to `to`, recorded as settled.
    async fn change_to(&self, db: &DatabaseConnection, to: Schedule) {
        change(db, self.id, self.announced, to, settled()).await
    }

    /// The announced schedule an hour later.
    fn moved(&self) -> Schedule {
        Schedule {
            date: self.announced.date + TimeDelta::hours(1),
            ..self.announced
        }
    }
}

/// A [`Seeded`] session moved an hour on, recorded as settled.
async fn changed_session(db: &DatabaseConnection, members: Members) -> Seeded {
    let seeded = Seeded::new(db, members).await;
    seeded.change_to(db, seeded.moved()).await;
    seeded
}

async fn notices(db: &DatabaseConnection, id: Id) -> BTreeMap<Id, DateTime<FixedOffset>> {
    entity_api::coaching_session_notice::find_by_coaching_session(db, id)
        .await
        .expect("the notices read")
        .into_iter()
        .map(|row| (row.user_id, row.notice_given_at))
        .collect()
}

/// Each written email's addressee, kind and subject key.
async fn written(db: &DatabaseConnection) -> BTreeSet<(Id, String, String)> {
    emails(db)
        .await
        .into_iter()
        .map(|(row, _)| (row.recipient_user_id, row.kind, row.subject_key))
        .collect()
}

/// The emails written, without their rows.
async fn written_emails(db: &DatabaseConnection) -> Vec<OutgoingEmail> {
    emails(db)
        .await
        .into_iter()
        .map(|(_, email)| email)
        .collect()
}

/// Runs one tick of the sweep under `config`; returns how many rows it handled of those it
/// tried.
async fn sweep(db: DatabaseConnection, config: Config) -> (crate::jobs::Context, (u64, u64)) {
    let ctx = job_context_with(db, config);
    let outcome = Sweep::from_config(&ctx.config)
        .run(&ctx)
        .await
        .expect("the tick runs");
    (ctx, (outcome.processed, outcome.attempted))
}

/// Claims every due row as one sweep would; fails unless exactly one is due.
async fn claim_one(db: &DatabaseConnection) -> Claimed {
    let claimed = coaching_session_pending_reschedule::claim_due(
        db,
        Utc::now().naive_utc(),
        Sweep::from_config(&composing()).timing,
        Id::new_v4(),
        BATCH,
    )
    .await
    .expect("the sweep claims");
    let [held] = <[_; 1]>::try_from(claimed).expect("one change is due");
    held
}

#[tokio::test]
async fn a_due_move_is_written_to_both_members_their_notice_stamped_and_settled() {
    within_time_limit(async {
        let db = database().await;
        let seeded = changed_session(&db, Members::Both).await;
        let before = notices(&db, seeded.id).await;

        let (ctx, outcome) = sweep(db, composing()).await;

        assert_eq!(outcome, (1, 1));
        let subject = format!("coaching_relationship/{}", seeded.relationship_id);
        let kind = "session_rescheduled".to_string();
        assert_eq!(
            written(&ctx.db).await,
            BTreeSet::from([
                (seeded.coach_id, kind.clone(), subject.clone()),
                (seeded.coachee_id, kind, subject),
            ])
        );
        for email in written_emails(&ctx.db).await {
            assert!(
                email.subject.contains("rescheduled to"),
                "{}",
                email.subject
            );
            let invite = calendar_part(&email);
            assert!(invite.contains("METHOD:REQUEST"), "{invite}");
            assert!(invite.contains(&format!("SEQUENCE:{SEQUENCE}")), "{invite}");
        }
        assert_eq!(
            session(&ctx.db, seeded.id).await.ical_sequence,
            SEQUENCE,
            "the edit already bumped it"
        );
        let after = notices(&ctx.db, seeded.id).await;
        assert!(after[&seeded.coach_id] > before[&seeded.coach_id]);
        assert!(after[&seeded.coachee_id] > before[&seeded.coachee_id]);
        assert_eq!(pending(&ctx.db, seeded.id).await, None);
    })
    .await;
}

#[tokio::test]
async fn a_length_only_change_names_no_earlier_start() {
    within_time_limit(async {
        let db = database().await;
        let seeded = Seeded::new(&db, Members::Both).await;
        seeded
            .change_to(
                &db,
                Schedule {
                    duration_minutes: 90,
                    ..seeded.announced
                },
            )
            .await;

        let (ctx, _) = sweep(db, composing()).await;

        let emails = written_emails(&ctx.db).await;
        assert_eq!(emails.len(), 2);
        for email in emails {
            assert!(email.subject.contains("now runs"), "{}", email.subject);
            assert!(!email.text.contains("Before:"), "{}", email.text);
        }
    })
    .await;
}

#[tokio::test]
async fn a_member_who_left_is_not_written_to_and_the_change_settles() {
    within_time_limit(async {
        let db = database().await;
        let seeded = changed_session(&db, Members::CoacheeOnly).await;
        let before = notices(&db, seeded.id).await;

        let (ctx, outcome) = sweep(db, composing()).await;

        assert_eq!(outcome, (1, 1));
        let recipients: Vec<Id> = written(&ctx.db)
            .await
            .into_iter()
            .map(|(to, ..)| to)
            .collect();
        assert_eq!(recipients, vec![seeded.coachee_id]);
        let after = notices(&ctx.db, seeded.id).await;
        assert_eq!(after[&seeded.coach_id], before[&seeded.coach_id]);
        assert!(after[&seeded.coachee_id] > before[&seeded.coachee_id]);
        assert_eq!(pending(&ctx.db, seeded.id).await, None);
    })
    .await;
}

#[tokio::test]
async fn a_change_no_member_is_owed_settles_without_an_email() {
    within_time_limit(async {
        let db = database().await;
        let seeded = changed_session(&db, Members::Neither).await;

        let (ctx, outcome) = sweep(db, composing()).await;

        assert_eq!(outcome, (1, 1));
        assert_eq!(written(&ctx.db).await, BTreeSet::new());
        assert_eq!(pending(&ctx.db, seeded.id).await, None);
    })
    .await;
}

#[tokio::test]
async fn a_stale_claim_writes_and_stamps_nothing() {
    within_time_limit(async {
        let db = database().await;
        let seeded = changed_session(&db, Members::Both).await;
        let before = notices(&db, seeded.id).await;
        let held = claim_one(&db).await;
        let stale = Claimed {
            claim_id: Id::new_v4(),
            ..held.clone()
        };

        crate::coaching_session::enqueue_reschedule(&db, &composing(), &stale)
            .await
            .expect("a stale claim is not an error");

        assert_eq!(written(&db).await, BTreeSet::new());
        assert_eq!(notices(&db, seeded.id).await, before);
        let row = pending(&db, seeded.id)
            .await
            .expect("the holder still owes it");
        assert_eq!(row.claim_id, Some(held.claim_id));
    })
    .await;
}

/// A change that lands after the claim is in the email, which describes the session as
/// locked, and stays pending from it, so the next tick has nothing left to tell.
#[tokio::test]
async fn a_change_after_the_claim_is_written_and_stays_pending_from_it() {
    within_time_limit(async {
        let db = database().await;
        let seeded = changed_session(&db, Members::Both).await;
        let held = claim_one(&db).await;
        let later = Schedule {
            date: seeded.moved().date + TimeDelta::hours(1),
            ..seeded.announced
        };
        change(
            &db,
            seeded.id,
            seeded.moved(),
            later,
            Utc::now().naive_utc(),
        )
        .await;

        crate::coaching_session::enqueue_reschedule(&db, &composing(), &held)
            .await
            .expect("the reschedule writes");

        let when = later
            .date
            .format("When: %A, %B %-d, %Y at %-I:%M %p")
            .to_string();
        let emails = written_emails(&db).await;
        assert_eq!(emails.len(), 2);
        assert!(emails.iter().all(|email| email.text.contains(&when)));
        let row = pending(&db, seeded.id)
            .await
            .expect("the later change stays pending");
        assert_eq!(row.claim_id, None);
        assert_eq!(
            (row.announced_date, row.announced_duration_minutes),
            (later.date, later.duration_minutes)
        );
    })
    .await;
}

/// With no frontend to link to no email composes, so the change is settled unsent rather
/// than retried.
#[tokio::test]
async fn a_change_that_cannot_compose_settles_unsent() {
    within_time_limit(async {
        let db = database().await;
        let seeded = changed_session(&db, Members::Both).await;
        let before = notices(&db, seeded.id).await;

        let (ctx, outcome) = sweep(db, Config::from_args(["test"])).await;

        assert_eq!(outcome, (1, 1));
        assert_eq!(written(&ctx.db).await, BTreeSet::new());
        assert_eq!(notices(&ctx.db, seeded.id).await, before);
        assert_eq!(pending(&ctx.db, seeded.id).await, None);
    })
    .await;
}

#[tokio::test]
async fn nothing_to_tell_settles_without_an_email() {
    within_time_limit(async {
        let db = database().await;
        let seeded = changed_session(&db, Members::Both).await;
        reschedule(&db, seeded.id, seeded.announced).await;

        let (ctx, outcome) = sweep(db, composing()).await;

        assert_eq!(outcome, (1, 1));
        assert_eq!(written(&ctx.db).await, BTreeSet::new());
        assert_eq!(pending(&ctx.db, seeded.id).await, None);
    })
    .await;
}

/// A change with nothing to tell loads none of the email, so a failing email load cannot
/// keep it from settling.
#[tokio::test]
async fn nothing_to_tell_settles_without_loading_the_email() {
    within_time_limit(async {
        let db = database().await;
        let seeded = changed_session(&db, Members::Both).await;
        reschedule(&db, seeded.id, seeded.announced).await;
        db.execute_unprepared("DROP TABLE user_roles")
            .await
            .expect("membership cannot be read");

        let (ctx, outcome) = sweep(db, composing()).await;

        assert_eq!(outcome, (1, 1));
        assert_eq!(pending(&ctx.db, seeded.id).await, None);
    })
    .await;
}

#[tokio::test]
async fn a_failed_write_is_retried_then_given_up() {
    within_time_limit(async {
        let db = database().await;
        let seeded = changed_session(&db, Members::Both).await;
        let before = notices(&db, seeded.id).await;
        db.execute_unprepared("DROP TABLE email_outbox")
            .await
            .expect("the outbox is gone");
        let ctx = job_context_with(db, composing());
        let sweep = Sweep::from_config(&ctx.config);

        let outcome = sweep.run(&ctx).await.expect("the tick runs");

        assert_eq!((outcome.processed, outcome.attempted), (0, 1));
        let row = pending(&ctx.db, seeded.id)
            .await
            .expect("the change is retried");
        assert_eq!((row.attempts, row.claim_id), (1, None));
        assert_eq!(notices(&ctx.db, seeded.id).await, before);

        for _ in 1..coaching_session_pending_reschedule::MAX_ATTEMPTS {
            sweep.run(&ctx).await.expect("the tick runs");
        }

        assert_eq!(pending(&ctx.db, seeded.id).await, None);
    })
    .await;
}

#[tokio::test]
async fn a_change_that_is_not_due_is_left_alone() {
    within_time_limit(async {
        let db = database().await;
        let seeded = Seeded::new(&db, Members::Both).await;
        change(
            &db,
            seeded.id,
            seeded.announced,
            seeded.moved(),
            Utc::now().naive_utc(),
        )
        .await;
        let before = pending(&db, seeded.id)
            .await
            .expect("the change is pending");

        let (ctx, outcome) = sweep(db, composing()).await;

        assert_eq!(outcome, (0, 0));
        assert_eq!(pending(&ctx.db, seeded.id).await, Some(before));
    })
    .await;
}

/// Composes invites, with Resend at `server` as the provider the sweep must not call.
fn inviting(server: &ServerGuard) -> Config {
    Config::from_args([
        "test",
        "--resend-api-key=test_api_key",
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
        let fixture = owing(&db, 5, true).await;
        coaching_session_pending_invite::queue(
            &db,
            &BTreeSet::from([Target::Series(fixture.sr1)]),
            &[fixture.user(Caller::Coach), fixture.user(Caller::Coachee)],
        )
        .await
        .expect("the series invites are owed");
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

/// Series `id` and each of its sessions' `SEQUENCE`.
async fn series_sequences(db: &DatabaseConnection, id: Id) -> (i32, Vec<i32>) {
    let series = coaching_session_series::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the series reads")
        .expect("the series exists");
    let mut sessions: Vec<i32> = coaching_sessions::Entity::find()
        .all(db)
        .await
        .expect("the sessions read")
        .into_iter()
        .filter(|session| session.coaching_session_series_id == Some(id))
        .map(|session| session.ical_sequence)
        .collect();
    sessions.sort();
    (series.ical_sequence, sessions)
}

/// The seeded fixture with series `series` of it owed to each of `callers`.
async fn owing_series(
    db: &DatabaseConnection,
    series: impl Fn(&Fixture) -> Id,
    callers: &[Caller],
) -> (Fixture, Id) {
    mirror_migration_defaults(db).await;
    let fixture = seed_fixture(db).await;
    let id = series(&fixture);
    let users: Vec<Id> = callers.iter().map(|caller| fixture.user(*caller)).collect();
    coaching_session_pending_invite::queue(db, &BTreeSet::from([Target::Series(id)]), &users)
        .await
        .expect("the invites are owed");
    (fixture, id)
}

#[tokio::test]
async fn an_owed_series_invite_is_written_and_settled_together() {
    within_time_limit(async {
        let mut server = Server::new_async().await;
        let resend = server
            .mock("POST", "/emails")
            .expect(0)
            .create_async()
            .await;
        let db = database().await;
        let (fixture, sr1) = owing_series(&db, |f| f.sr1, &[Caller::Coach, Caller::Coachee]).await;
        let ctx = job_context_with(db, inviting(&server));

        Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        let mut written = outboxed(&ctx.db).await;
        written.sort();
        let mut expected = [Caller::Coach, Caller::Coachee]
            .map(|caller| ("series_scheduled".to_string(), fixture.user(caller)))
            .to_vec();
        expected.sort();
        assert_eq!(written, expected);
        assert_eq!(owed_rows(&ctx.db).await, Vec::new());
        assert_eq!(
            series_sequences(&ctx.db, sr1).await,
            (2, vec![2, 2]),
            "each copy goes out under its own SEQUENCE, its members raised to it"
        );
        for (_, email) in emails(&*ctx.db).await {
            let invite = calendar_part(&email);
            assert!(
                invite.contains(&format!("UID:{sr1}@myrefactor.com")),
                "{invite}"
            );
            assert!(invite.contains("RRULE:FREQ=WEEKLY"), "{invite}");
        }
        resend.assert_async().await;
    })
    .await;
}

#[tokio::test]
async fn a_stale_claim_on_an_owed_series_invite_writes_nothing() {
    within_time_limit(async {
        let db = database().await;
        let (_, sr1) = owing_series(&db, |f| f.sr1, &[Caller::Coach]).await;
        let claimed = coaching_session_pending_invite::claim(
            &db,
            Utc::now().naive_utc(),
            Id::new_v4(),
            INVITE_BATCH,
        )
        .await
        .expect("the sweep claims");
        let [held] = <[_; 1]>::try_from(claimed).expect("one invite is owed");
        let before = series_sequences(&db, sr1).await;
        let stale = coaching_session_pending_invite::Claimed {
            claim_id: Id::new_v4(),
            ..held
        };

        crate::coaching_session_series::enqueue_owed_invite(
            &db,
            &inviting(&Server::new_async().await),
            sr1,
            &stale,
        )
        .await
        .expect("a stale claim is not an error");

        assert_eq!(outboxed(&db).await, Vec::new());
        assert_eq!(series_sequences(&db, sr1).await, before);
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
async fn an_owed_series_invite_that_cannot_compose_is_settled_unsent() {
    within_time_limit(async {
        let db = database().await;
        let (_, sr1) = owing_series(&db, |f| f.sr1, &[Caller::Coach]).await;
        let before = series_sequences(&db, sr1).await;
        let ctx = job_context_with(db, Config::from_args(["test"]));

        Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        assert_eq!(owed_rows(&ctx.db).await, Vec::new());
        assert_eq!(outboxed(&ctx.db).await, Vec::new());
        assert_eq!(series_sequences(&ctx.db, sr1).await, before);
    })
    .await;
}

/// A series with nothing left to attend, and a participant who left the organization, are
/// owed no invite: the row settles with nothing written or bumped.
#[tokio::test]
async fn an_owed_series_invite_nobody_is_owed_settles_unsent() {
    within_time_limit(async {
        // SR1 moved into the past for COACH; SR3 for REMOVED, who holds no role.
        for (case, (caller, into_the_past)) in [(Caller::Coach, true), (Caller::Removed, false)]
            .into_iter()
            .enumerate()
        {
            let db = database().await;
            let series = |f: &Fixture| if into_the_past { f.sr1 } else { f.sr3 };
            let (_, id) = owing_series(&db, series, &[caller]).await;
            if into_the_past {
                move_series_into_the_past(&db, id).await;
            }
            let before = series_sequences(&db, id).await;
            let ctx = job_context_with(db, composing());

            Sweep::from_config(&ctx.config)
                .run(&ctx)
                .await
                .expect("the tick runs");

            assert_eq!(owed_rows(&ctx.db).await, Vec::new(), "case {case}");
            assert_eq!(outboxed(&ctx.db).await, Vec::new(), "case {case}");
            assert_eq!(series_sequences(&ctx.db, id).await, before, "case {case}");
        }
    })
    .await;
}

/// An owed series invite whose write fails is released to retry, its bump rolled back.
#[tokio::test]
async fn an_owed_series_invite_whose_write_fails_is_retried_unbumped() {
    within_time_limit(async {
        let db = database().await;
        let (_, sr1) = owing_series(&db, |f| f.sr1, &[Caller::Coach]).await;
        let before = series_sequences(&db, sr1).await;
        db.execute_unprepared("DROP TABLE email_outbox")
            .await
            .expect("the outbox is gone");
        let ctx = job_context_with(db, composing());

        let outcome = Sweep::from_config(&ctx.config)
            .run(&ctx)
            .await
            .expect("the tick runs");

        assert_eq!((outcome.processed, outcome.attempted), (0, 1));
        assert_eq!(owed_rows(&ctx.db).await, vec![(1, false)]);
        assert_eq!(series_sequences(&ctx.db, sr1).await, before);
    })
    .await;
}
