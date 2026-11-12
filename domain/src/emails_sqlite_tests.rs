//! Who relationship emails reach, with membership read from real `user_roles` rows.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use chrono::{NaiveDateTime, TimeDelta, Timelike, Utc};
use entity::coaching_session_series as series_entity;
use mockito::{Mock, Server, ServerGuard};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

use super::*;
use crate::test_utils::sqlite::{
    database, seed_membership, seed_organization, seed_user, within_time_limit,
};

const SESSION_CANCELLED: &str = "session_cancel_tpl";

fn sending(server: &ServerGuard) -> Config {
    Config::from_args([
        "test",
        "--resend-api-key=test_api_key",
        "--session-scheduled-email-template-id=session_scheduled_tpl",
        &format!("--session-cancelled-email-template-id={SESSION_CANCELLED}"),
        "--recurring-sessions-rescheduled-email-template-id=series_rescheduled_tpl",
        "--recurring-sessions-cancelled-email-template-id=series_cancel_tpl",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={}", server.url()),
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
    })
    .await;
}
