//! What a session edit records as an unannounced reschedule, and what clears it, against a
//! real SQL engine.

use chrono::{NaiveDateTime, TimeDelta, Timelike, Utc};
use entity_api::{coaching_session_pending_reschedule, mutate};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, IntoActiveModel, Set, Value};

use crate::error::{DomainErrorKind, EntityErrorKind, InternalErrorKind};
use crate::test_utils::sqlite::{
    database, organization_of, seed_coaching_session, seed_membership, seed_user, within_time_limit,
};
use crate::{coaching_session_pending_reschedules, coaching_sessions, Id};

#[derive(Debug)]
struct Params(mutate::UpdateMap);

impl mutate::IntoUpdateMap for Params {
    fn into_update_map(self) -> mutate::UpdateMap {
        self.0
    }
}

fn set(column: &str, value: Value) -> Params {
    let mut map = mutate::UpdateMap::new();
    map.insert(column.into(), Some(value));
    Params(map)
}

fn date(value: NaiveDateTime) -> Params {
    set("date", Value::ChronoDateTime(Some(value)))
}

fn duration(minutes: i16) -> Params {
    set("duration_minutes", Value::SmallInt(Some(minutes)))
}

fn title(value: &str) -> Params {
    set("title", Value::String(Some(value.to_string())))
}

/// A session a week out, 60 minutes long, whose one participant holds no role in its
/// organization; returns its id, start and participant.
async fn upcoming_session_without_members(db: &DatabaseConnection) -> (Id, NaiveDateTime, Id) {
    let (session_id, user_id) = seed_coaching_session(db).await;
    let start = (Utc::now() + TimeDelta::days(7))
        .naive_utc()
        .with_nanosecond(0)
        .expect("whole seconds");
    let mut session = read(db, session_id).await.into_active_model();
    session.date = Set(start);
    session.update(db).await.expect("the session moves");
    (session_id, start, user_id)
}

/// A session a week out, 60 minutes long; returns its id and start.
async fn upcoming_session(db: &DatabaseConnection) -> (Id, NaiveDateTime) {
    let (session_id, start, user_id) = upcoming_session_without_members(db).await;
    seed_membership(db, user_id, organization_of(db, session_id).await).await;
    (session_id, start)
}

async fn read(db: &DatabaseConnection, session_id: Id) -> coaching_sessions::Model {
    coaching_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("the session reads")
        .expect("the session exists")
}

async fn edit(db: &DatabaseConnection, session_id: Id, params: Params) -> coaching_sessions::Model {
    crate::coaching_session::update(db, session_id, Id::new_v4(), params)
        .await
        .expect("the edit succeeds")
}

async fn pending(
    db: &DatabaseConnection,
    session_id: Id,
) -> Option<coaching_session_pending_reschedules::Model> {
    coaching_session_pending_reschedule::find(db, session_id)
        .await
        .expect("the pending row reads")
}

#[tokio::test]
async fn moving_a_session_records_what_participants_were_last_told() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        let moved = edit(&db, session_id, date(start + TimeDelta::hours(1))).await;
        assert_eq!(moved.date, start + TimeDelta::hours(1));

        let row = pending(&db, session_id).await.expect("the move is pending");
        assert_eq!(row.announced_date, start);
        assert_eq!(row.announced_duration_minutes, 60);
        assert_eq!(row.revision, 0);
        assert_eq!(row.claim_id, None);
    })
    .await;
}

#[tokio::test]
async fn resizing_a_session_records_a_pending_change() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        edit(&db, session_id, duration(45)).await;

        let row = pending(&db, session_id)
            .await
            .expect("the resize is pending");
        assert_eq!(row.announced_date, start);
        assert_eq!(row.announced_duration_minutes, 60);
    })
    .await;
}

#[tokio::test]
async fn further_moves_keep_the_first_announced_schedule() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        edit(&db, session_id, date(start + TimeDelta::hours(1))).await;
        edit(&db, session_id, date(start + TimeDelta::hours(2))).await;

        let row = pending(&db, session_id)
            .await
            .expect("the moves are pending");
        assert_eq!(row.announced_date, start);
        assert_eq!(row.revision, 1);
    })
    .await;
}

#[tokio::test]
async fn moving_back_to_the_announced_schedule_leaves_nothing_pending() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        edit(&db, session_id, date(start + TimeDelta::hours(1))).await;
        edit(&db, session_id, date(start)).await;

        assert_eq!(pending(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn moving_a_session_records_the_acting_user_as_scheduler() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;
        let acting_user = seed_user(&db).await;

        crate::coaching_session::update(
            &db,
            session_id,
            acting_user,
            date(start + TimeDelta::hours(1)),
        )
        .await
        .expect("the edit succeeds");

        assert_eq!(
            read(&db, session_id).await.scheduled_by_user_id,
            acting_user
        );
    })
    .await;
}

#[tokio::test]
async fn a_title_edit_leaves_the_scheduler_as_it_was() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, _) = upcoming_session(&db).await;

        let before = read(&db, session_id).await.scheduled_by_user_id;

        crate::coaching_session::update(&db, session_id, seed_user(&db).await, title("Renamed"))
            .await
            .expect("the edit succeeds");

        assert_eq!(read(&db, session_id).await.scheduled_by_user_id, before);
    })
    .await;
}

fn meeting_url(value: Option<&str>) -> Params {
    set("meeting_url", Value::String(value.map(str::to_string)))
}

/// An upcoming session with a link its coach made; returns its id and the coach.
async fn linked_session(db: &DatabaseConnection) -> (Id, Id) {
    let (session_id, _, coach_id) = upcoming_session_without_members(db).await;
    seed_membership(db, coach_id, organization_of(db, session_id).await).await;
    let mut session = read(db, session_id).await.into_active_model();
    session.meeting_url = Set(Some("https://meet.example/coach".to_string()));
    session.meeting_url_created_by_user_id = Set(Some(coach_id));
    session.update(db).await.expect("the link is set");
    (session_id, coach_id)
}

async fn edit_as(db: &DatabaseConnection, session_id: Id, acting_user: Id, params: Params) {
    crate::coaching_session::update(db, session_id, acting_user, params)
        .await
        .expect("the edit succeeds");
}

#[tokio::test]
async fn setting_a_meeting_link_records_the_acting_user_as_its_creator() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, _) = linked_session(&db).await;
        let acting_user = seed_user(&db).await;

        edit_as(
            &db,
            session_id,
            acting_user,
            meeting_url(Some("https://meet.example/pasted")),
        )
        .await;

        assert_eq!(
            read(&db, session_id).await.meeting_url_created_by_user_id,
            Some(acting_user)
        );
    })
    .await;
}

#[tokio::test]
async fn clearing_the_meeting_link_clears_its_creator() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, _) = linked_session(&db).await;

        edit_as(&db, session_id, seed_user(&db).await, meeting_url(None)).await;

        let session = read(&db, session_id).await;
        assert_eq!(session.meeting_url, None);
        assert_eq!(session.meeting_url_created_by_user_id, None);
    })
    .await;
}

#[tokio::test]
async fn a_title_edit_leaves_the_link_creator_as_it_was() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, coach_id) = linked_session(&db).await;

        edit_as(&db, session_id, seed_user(&db).await, title("Renamed")).await;

        assert_eq!(
            read(&db, session_id).await.meeting_url_created_by_user_id,
            Some(coach_id)
        );
    })
    .await;
}

#[tokio::test]
async fn a_title_edit_records_nothing() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, _) = upcoming_session(&db).await;

        edit(&db, session_id, title("Renamed")).await;

        assert_eq!(pending(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn an_edit_that_changes_nothing_records_nothing() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        edit(&db, session_id, date(start)).await;

        assert_eq!(pending(&db, session_id).await, None);
    })
    .await;
}

#[tokio::test]
async fn an_edit_after_the_email_was_read_stays_pending_from_what_was_sent() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start) = upcoming_session(&db).await;

        let sent = edit(&db, session_id, date(start + TimeDelta::hours(1))).await;
        edit(&db, session_id, date(start + TimeDelta::hours(2))).await;
        crate::coaching_session::record_announced(&db, &sent).await;

        let row = pending(&db, session_id)
            .await
            .expect("the newer edit stays pending");
        assert_eq!(row.announced_date, sent.date);
    })
    .await;
}

/// The `details` of a 409, or `None` for any other error.
fn conflict_details(error: &crate::error::Error) -> Option<&serde_json::Value> {
    match &error.error_kind {
        DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Conflict {
            details,
            ..
        })) => details.as_ref(),
        _ => None,
    }
}

#[tokio::test]
async fn moving_a_session_whose_coachee_left_the_organization_is_refused() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, start, coachee_id) = upcoming_session_without_members(&db).await;
        let before = read(&db, session_id).await;

        for params in [date(start + TimeDelta::hours(1)), duration(45)] {
            let refused = crate::coaching_session::update(&db, session_id, Id::new_v4(), params)
                .await
                .expect_err("a move is refused");

            let details = conflict_details(&refused).expect("a 409 with details");
            assert_eq!(details["reason"], "participant_not_a_member");
            assert_eq!(details["user_id"], serde_json::json!(coachee_id));
            assert_eq!(
                details["organization_id"],
                serde_json::json!(organization_of(&db, session_id).await)
            );
            assert_eq!(read(&db, session_id).await, before);
            assert_eq!(pending(&db, session_id).await, None);
        }
    })
    .await;
}

#[tokio::test]
async fn retitling_a_session_whose_coachee_left_the_organization_still_saves() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, _, _) = upcoming_session_without_members(&db).await;

        let retitled = edit(&db, session_id, title("Quarterly review")).await;

        assert_eq!(retitled.title.as_deref(), Some("Quarterly review"));
        assert_eq!(
            read(&db, session_id).await.title.as_deref(),
            Some("Quarterly review")
        );
    })
    .await;
}

#[tokio::test]
async fn deleting_a_session_whose_coachee_left_the_organization_is_allowed() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, _, _) = upcoming_session_without_members(&db).await;

        crate::coaching_session::delete(&db, &service::config::Config::default(), session_id)
            .await
            .expect("the delete succeeds");

        assert_eq!(
            coaching_sessions::Entity::find_by_id(session_id)
                .one(&db)
                .await
                .expect("the session reads"),
            None
        );
    })
    .await;
}
