//! Who relationship emails reach, with membership read from real `user_roles` rows.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use chrono::{NaiveDateTime, TimeDelta, Timelike, Utc};
use entity::coaching_session_series as series_entity;
use mockito::{Matcher, Mock, Server, ServerGuard};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

use super::*;
use crate::coaching_session::{Frequency, Recurrence};
use crate::events::EventPublisher;
use crate::test_utils::sqlite::{
    database, mirror_migration_defaults, seed_membership, seed_organization, seed_user,
    within_time_limit,
};

const SESSION_CANCELLED: &str = "session_cancel_tpl";

const SERIES_RESCHEDULED: &str = "series_rescheduled_tpl";

fn sending(server: &ServerGuard) -> Config {
    Config::from_args([
        "test",
        "--resend-api-key=test_api_key",
        "--session-scheduled-email-template-id=session_scheduled_tpl",
        "--recurring-sessions-scheduled-email-template-id=series_scheduled_tpl",
        &format!("--session-cancelled-email-template-id={SESSION_CANCELLED}"),
        &format!("--recurring-sessions-rescheduled-email-template-id={SERIES_RESCHEDULED}"),
        "--recurring-sessions-cancelled-email-template-id=series_cancel_tpl",
        "--action-assigned-email-template-id=action_assigned_tpl",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={}", server.url()),
        "--tiptap-auth-key=test-auth-key",
        &format!("--tiptap-url={}", server.url()),
    ])
}

/// Each Resend request's addressee and template.
#[derive(Clone, Default)]
struct Sent(Arc<Mutex<Vec<(String, String)>>>);

impl Sent {
    async fn capture(&self, server: &mut ServerGuard) -> Mock {
        let seen = self.0.clone();
        server
            .mock("POST", "/emails")
            .match_request(move |request| {
                let body = request
                    .body()
                    .ok()
                    .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok())
                    .unwrap_or_default();
                let to = body["to"][0].as_str().unwrap_or_default();
                let address = to
                    .rsplit_once('<')
                    .and_then(|(_, rest)| rest.strip_suffix('>'))
                    .unwrap_or(to);
                let template = body["template"]["id"].as_str().unwrap_or_default();
                seen.lock()
                    .expect("the capture lock")
                    .push((address.to_string(), template.to_string()));
                true
            })
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .create_async()
            .await
    }

    fn addressees(&self) -> BTreeSet<String> {
        self.all().into_iter().map(|(to, _)| to).collect()
    }

    fn templates_to(&self, address: &str) -> Vec<String> {
        self.all()
            .into_iter()
            .filter(|(to, _)| to == address)
            .map(|(_, template)| template)
            .collect()
    }

    fn all(&self) -> Vec<(String, String)> {
        self.0.lock().expect("the capture lock").clone()
    }
}

/// A coach and coachee in one organization with an upcoming standalone session. Neither
/// holds a role until the test seeds one.
struct Seeded {
    coach: users::Model,
    coachee: users::Model,
    relationship: coaching_relationships::Model,
    session: coaching_sessions::Model,
}

fn next_week() -> NaiveDateTime {
    (Utc::now() + TimeDelta::days(7))
        .naive_utc()
        .with_nanosecond(0)
        .expect("whole seconds")
}

async fn user_row(db: &DatabaseConnection, id: Id) -> users::Model {
    users::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the user reads")
        .expect("the user exists")
}

fn session_row(
    relationship_id: Id,
    series_id: Option<Id>,
    date: NaiveDateTime,
) -> coaching_sessions::ActiveModel {
    let now = Utc::now();
    coaching_sessions::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_relationship_id: Set(relationship_id),
        coaching_session_series_id: Set(series_id),
        ical_sequence: Set(0),
        ical_recurrence_id: Set(series_id.map(|_| date)),
        collab_document_name: Set(None),
        date: Set(date),
        duration_minutes: Set(60),
        title: Set(None),
        meeting_url: Set(None),
        provider: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        hydrated_at: Set(None),
        notice_given_at: Set(now.into()),
    }
}

async fn seed(db: &DatabaseConnection) -> Seeded {
    let (coach_id, coachee_id) = (seed_user(db).await, seed_user(db).await);
    let organization_id = seed_organization(db).await;
    let now = Utc::now();
    let relationship_id = Id::new_v4();
    let relationship = coaching_relationships::ActiveModel {
        id: Set(relationship_id),
        organization_id: Set(organization_id),
        coach_id: Set(coach_id),
        coachee_id: Set(coachee_id),
        slug: Set(format!("relationship-{relationship_id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the relationship is seeded");
    let session = session_row(relationship_id, None, next_week())
        .insert(db)
        .await
        .expect("the session is seeded");
    Seeded {
        coach: user_row(db, coach_id).await,
        coachee: user_row(db, coachee_id).await,
        relationship,
        session,
    }
}

impl Seeded {
    async fn grant_role(&self, db: &DatabaseConnection, user_id: Id) {
        seed_membership(db, user_id, self.relationship.organization_id).await;
    }

    fn both(&self) -> BTreeSet<String> {
        BTreeSet::from([self.coach.email.clone(), self.coachee.email.clone()])
    }

    /// A weekly two-session series on the relationship, starting next week.
    async fn series(
        &self,
        db: &DatabaseConnection,
    ) -> (series_entity::Model, Vec<coaching_sessions::Model>) {
        let start = next_week();
        let now = Utc::now();
        let series = series_entity::ActiveModel {
            id: Set(Id::new_v4()),
            coaching_relationship_id: Set(self.relationship.id),
            rule: Set(serde_json::json!({
                "start_at": start,
                "recurrence": { "frequency": "weekly", "interval": 1, "count": 2 },
                "duration_minutes": 60,
                "timezone": "UTC",
            })),
            ical_sequence: Set(0),
            created_by_user_id: Set(self.coach.id),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(db)
        .await
        .expect("the series is seeded");
        let mut sessions = Vec::new();
        for week in 0..2 {
            sessions.push(
                session_row(
                    self.relationship.id,
                    Some(series.id),
                    start + TimeDelta::weeks(week),
                )
                .insert(db)
                .await
                .expect("the series session is seeded"),
            );
        }
        (series, sessions)
    }
}

async fn cancel_session(seeded: &Seeded, db: &DatabaseConnection) -> BTreeSet<String> {
    let mut server = Server::new_async().await;
    let sent = Sent::default();
    let _mock = sent.capture(&mut server).await;
    notify_session_cancelled(db, &sending(&server), &seeded.session).await;
    sent.addressees()
}

/// M9-01
#[tokio::test]
async fn single_cancel_with_the_coachee_holding_no_role_reaches_both() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed(&db).await;
        seeded.grant_role(&db, seeded.coach.id).await;

        assert_eq!(cancel_session(&seeded, &db).await, seeded.both());
    })
    .await;
}

/// M9-02
#[tokio::test]
async fn single_cancel_with_the_coach_holding_no_role_reaches_both() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed(&db).await;
        seeded.grant_role(&db, seeded.coachee.id).await;

        assert_eq!(cancel_session(&seeded, &db).await, seeded.both());
    })
    .await;
}

/// M9-03
#[tokio::test]
async fn series_cancel_with_the_coachee_holding_no_role_reaches_both() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed(&db).await;
        seeded.grant_role(&db, seeded.coach.id).await;
        let (series, sessions) = seeded.series(&db).await;
        let mut server = Server::new_async().await;
        let sent = Sent::default();
        let _mock = sent.capture(&mut server).await;

        notify_recurring_sessions_cancelled(&db, &sending(&server), &series, &sessions).await;

        assert_eq!(sent.addressees(), seeded.both());
    })
    .await;
}

/// M9-04: the seed is a real removal, so a members-only send skips the coachee.
#[tokio::test]
async fn session_scheduled_with_the_coachee_holding_no_role_reaches_only_the_coach() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed(&db).await;
        seeded.grant_role(&db, seeded.coach.id).await;
        let organization = organization::find_by_id(&db, seeded.relationship.organization_id)
            .await
            .expect("the organization reads");
        let member_ids = notify_member_ids(&db, &seeded.relationship)
            .await
            .expect("membership reads");
        let mut server = Server::new_async().await;
        let sent = Sent::default();
        let _mock = sent.capture(&mut server).await;

        notify_session_scheduled(
            &db,
            &sending(&server),
            &seeded.session,
            &seeded.relationship,
            &organization,
            &member_ids,
        )
        .await;

        assert_eq!(
            sent.addressees(),
            BTreeSet::from([seeded.coach.email.clone()])
        );
    })
    .await;
}

/// M9-05: a series edit that strands a legacy standalone event cancels it for a coachee
/// holding no role, on its own session-cancelled email.
#[tokio::test]
async fn a_series_edit_cancels_an_orphan_for_a_coachee_holding_no_role() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed(&db).await;
        seeded.grant_role(&db, seeded.coach.id).await;
        let (series, sessions) = seeded.series(&db).await;
        let orphan = coaching_sessions::Model {
            ical_recurrence_id: None,
            ical_sequence: 1,
            ..sessions[0].clone()
        };
        let member_ids = notify_member_ids(&db, &seeded.relationship)
            .await
            .expect("membership reads");
        let mut server = Server::new_async().await;
        let sent = Sent::default();
        let _mock = sent.capture(&mut server).await;

        notify_recurring_sessions_rescheduled(
            &db,
            &sending(&server),
            &series,
            PreviousSeries(&series),
            &sessions,
            ReplacedSessions(std::slice::from_ref(&orphan)),
            &member_ids,
        )
        .await;

        assert_eq!(
            sent.templates_to(&seeded.coachee.email),
            vec![SESSION_CANCELLED.to_string()]
        );
        assert_eq!(
            sent.templates_to(&seeded.coach.email),
            vec![SERIES_RESCHEDULED.to_string()]
        );
    })
    .await;
}

/// MAIL-04: a session resend skips a coach holding no role. The router refuses that coach,
/// so only a call made here reaches the member filter.
#[tokio::test]
async fn session_resend_with_the_coach_holding_no_role_reaches_only_the_coachee() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed(&db).await;
        seeded.grant_role(&db, seeded.coachee.id).await;
        let mut server = Server::new_async().await;
        let sent = Sent::default();
        let _mock = sent.capture(&mut server).await;

        crate::coaching_session::resend_invite(&db, &sending(&server), seeded.session.id)
            .await
            .expect("the resend succeeds");

        assert_eq!(
            sent.addressees(),
            BTreeSet::from([seeded.coachee.email.clone()])
        );
    })
    .await;
}

/// MAIL-09: a series resend skips a coach holding no role.
#[tokio::test]
async fn series_resend_with_the_coach_holding_no_role_reaches_only_the_coachee() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed(&db).await;
        seeded.grant_role(&db, seeded.coachee.id).await;
        let (series, _) = seeded.series(&db).await;
        let mut server = Server::new_async().await;
        let sent = Sent::default();
        let _mock = sent.capture(&mut server).await;

        crate::coaching_session_series::resend_invite(&db, &sending(&server), series.id)
            .await
            .expect("the resend succeeds");

        assert_eq!(
            sent.addressees(),
            BTreeSet::from([seeded.coachee.email.clone()])
        );
    })
    .await;
}

/// The members-only emails a notify entry point sends about a series or its sessions.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MembersOnly {
    SessionScheduled,
    SeriesScheduled,
    SeriesRescheduled,
    SeriesFollowing,
}

/// Which participants hold a role.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Members {
    Both,
    CoachOnly,
    CoacheeOnly,
    Neither,
}

/// Sends `email` to whoever holds a role; returns who was mailed, and for the "this and
/// following" reschedule, what it did for the coachee.
async fn send_members_only(
    email: MembersOnly,
    seeded: &Seeded,
    db: &DatabaseConnection,
) -> (BTreeSet<String>, Option<Delivery>) {
    let (series, sessions) = seeded.series(db).await;
    let organization = organization::find_by_id(db, seeded.relationship.organization_id)
        .await
        .expect("the organization reads");
    let member_ids = notify_member_ids(db, &seeded.relationship)
        .await
        .expect("membership reads");
    let mut server = Server::new_async().await;
    let config = sending(&server);
    let sent = Sent::default();
    let _mock = sent.capture(&mut server).await;

    let mut told = None;
    match email {
        MembersOnly::SessionScheduled => {
            notify_session_scheduled(
                db,
                &config,
                &seeded.session,
                &seeded.relationship,
                &organization,
                &member_ids,
            )
            .await;
        }
        MembersOnly::SeriesScheduled => {
            notify_recurring_sessions_scheduled(
                db,
                &config,
                &series,
                &sessions,
                &seeded.relationship,
                &member_ids,
            )
            .await;
        }
        MembersOnly::SeriesRescheduled => {
            notify_recurring_sessions_rescheduled(
                db,
                &config,
                &series,
                PreviousSeries(&series),
                &sessions,
                ReplacedSessions(&[]),
                &member_ids,
            )
            .await;
        }
        MembersOnly::SeriesFollowing => {
            told = Some(
                notify_recurring_sessions_rescheduled_following(
                    db,
                    &config,
                    &FollowingRescheduleNotice {
                        series: &series,
                        sessions: &sessions,
                        previous_series: PreviousSeries(&series),
                        previous_start: sessions[0].date,
                        replaced: ReplacedSessions(&[]),
                        split_from: None,
                        previous_overrides: &[],
                        member_ids: &member_ids,
                    },
                )
                .await,
            );
        }
    }
    (sent.addressees(), told)
}

/// A fresh database seeded with `members` holding a role. It carries the migrations'
/// column defaults, which the create calls rely on.
async fn seeded_with(members: Members) -> (DatabaseConnection, Seeded) {
    let db = database().await;
    mirror_migration_defaults(&db).await;
    let seeded = seed(&db).await;
    if matches!(members, Members::Both | Members::CoachOnly) {
        seeded.grant_role(&db, seeded.coach.id).await;
    }
    if matches!(members, Members::Both | Members::CoacheeOnly) {
        seeded.grant_role(&db, seeded.coachee.id).await;
    }
    (db, seeded)
}

/// MAIL-01, MAIL-05, MAIL-06 and MAIL-07: each reaches only the participants holding a
/// role, and nobody when neither does.
#[tokio::test]
async fn members_only_emails_reach_only_participants_holding_a_role() {
    within_time_limit(async {
        // Checked through the calls that send them, in the tests named beside each.
        let elsewhere = [
            // session_scheduled_with_the_coachee_holding_no_role_reaches_only_the_coach
            (MembersOnly::SessionScheduled, Members::CoachOnly),
            // session_create_with_the_coach_holding_no_role_reaches_only_the_coachee
            (MembersOnly::SessionScheduled, Members::CoacheeOnly),
            // series_create_with_the_coach_holding_no_role_reaches_only_the_coachee
            (MembersOnly::SeriesScheduled, Members::CoacheeOnly),
        ];

        for email in [
            MembersOnly::SessionScheduled,
            MembersOnly::SeriesScheduled,
            MembersOnly::SeriesRescheduled,
            MembersOnly::SeriesFollowing,
        ] {
            for members in [
                Members::Both,
                Members::CoachOnly,
                Members::CoacheeOnly,
                Members::Neither,
            ] {
                if elsewhere.contains(&(email, members)) {
                    continue;
                }
                let (db, seeded) = seeded_with(members).await;
                let expected = match members {
                    Members::Both => seeded.both(),
                    Members::CoachOnly => BTreeSet::from([seeded.coach.email.clone()]),
                    Members::CoacheeOnly => BTreeSet::from([seeded.coachee.email.clone()]),
                    Members::Neither => BTreeSet::new(),
                };

                let (mailed, _) = send_members_only(email, &seeded, &db).await;

                assert_eq!(mailed, expected, "{email:?} with {members:?}");
            }
        }
    })
    .await;
}

/// MAIL-07: a "this and following" reschedule reports a coachee holding no role as skipped,
/// not failed.
#[tokio::test]
async fn a_following_reschedule_reports_a_coachee_holding_no_role_as_not_a_member() {
    within_time_limit(async {
        for (members, expected) in [
            (Members::CoachOnly, Delivery::NoLongerAMember),
            (Members::Both, Delivery::Sent),
        ] {
            let (db, seeded) = seeded_with(members).await;

            let (_, told) = send_members_only(MembersOnly::SeriesFollowing, &seeded, &db).await;

            assert_eq!(told, Some(expected), "{members:?}");
        }
    })
    .await;
}

/// MAIL-01: creating a session skips a coach holding no role. The router refuses that
/// coach, so only a call made here reaches the member filter.
#[tokio::test]
async fn session_create_with_the_coach_holding_no_role_reaches_only_the_coachee() {
    within_time_limit(async {
        let (db, seeded) = seeded_with(Members::CoacheeOnly).await;
        let mut server = Server::new_async().await;
        let sent = Sent::default();
        let _mock = sent.capture(&mut server).await;
        let _tiptap = server
            .mock("POST", Matcher::Regex("^/api/documents/".to_string()))
            .with_status(200)
            .create_async()
            .await;
        let session = coaching_sessions::Model {
            id: Id::new_v4(),
            date: seeded.session.date + TimeDelta::days(1),
            ..seeded.session.clone()
        };

        crate::coaching_session::create(
            &db,
            &sending(&server),
            &EventPublisher::new(),
            session,
            None,
        )
        .await
        .expect("the session is created");

        assert_eq!(
            sent.addressees(),
            BTreeSet::from([seeded.coachee.email.clone()])
        );
    })
    .await;
}

/// MAIL-05: creating a series skips a coach holding no role.
#[tokio::test]
async fn series_create_with_the_coach_holding_no_role_reaches_only_the_coachee() {
    within_time_limit(async {
        let (db, seeded) = seeded_with(Members::CoacheeOnly).await;
        let mut server = Server::new_async().await;
        let sent = Sent::default();
        let _mock = sent.capture(&mut server).await;
        let weekly_twice = Recurrence {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekdays: None,
            count: Some(2),
            until: None,
        };

        crate::coaching_session_series::create_with_sessions(
            &db,
            &sending(&server),
            seeded.relationship.id,
            seeded.coach.id,
            next_week() + TimeDelta::days(1),
            weekly_twice,
            None,
        )
        .await
        .expect("the series is created");

        assert_eq!(
            sent.addressees(),
            BTreeSet::from([seeded.coachee.email.clone()])
        );
    })
    .await;
}

/// MAIL-11: an action assigned to both participants mails only the one holding a role.
#[tokio::test]
async fn action_assigned_reaches_only_an_assignee_holding_a_role() {
    within_time_limit(async {
        let (db, seeded) = seeded_with(Members::CoachOnly).await;
        let mut server = Server::new_async().await;
        let sent = Sent::default();
        let _mock = sent.capture(&mut server).await;
        let now = Utc::now().fixed_offset();
        let action = actions::Model {
            id: Id::new_v4(),
            coaching_session_id: seeded.session.id,
            goal_id: None,
            user_id: seeded.coach.id,
            body: Some("Write it down".into()),
            due_by: None,
            status: entity::status::Status::NotStarted,
            status_changed_at: now,
            created_at: now,
            updated_at: now,
        };

        notify_action_assigned(
            &db,
            &sending(&server),
            &[seeded.coach.id, seeded.coachee.id],
            &seeded.coach,
            &action,
        )
        .await;

        assert_eq!(
            sent.addressees(),
            BTreeSet::from([seeded.coach.email.clone()])
        );
    })
    .await;
}

/// JOB-01 and MAIL-10: a reminder's recipient is re-read as a member just before the send.
#[tokio::test]
async fn a_reminder_recipient_is_mailed_only_while_holding_a_role() {
    within_time_limit(async {
        let (db, seeded) = seeded_with(Members::CoachOnly).await;
        let coachee = seeded.coachee.id;
        let still_notified = || async {
            recipient_still_notified(&db, &seeded.relationship, coachee)
                .await
                .expect("membership reads")
        };

        assert!(!still_notified().await);

        seeded.grant_role(&db, coachee).await;
        assert!(still_notified().await);
    })
    .await;
}
