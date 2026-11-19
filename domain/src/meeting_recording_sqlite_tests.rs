//! Starting a recording against a real SQL engine: the claim is the one arbiter of which
//! start sends a bot, so a session never gets two.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use entity::{coaching_relationships, meeting_recording};
use events::EventHandler;
use meeting_ai::types::recording::{Config as BotConfig, Filters, Info, Status as BotStatus};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use tokio::sync::oneshot;

use super::*;
use crate::test_utils::sqlite::{
    database, mirror_migration_defaults, seed_coaching_session, seed_membership, seed_user,
    within_time_limit,
};

const MEETING_URL: &str = "https://meet.google.com/abc-defg-hij";

/// Every status a recording is still active in.
const ACTIVE_STATUSES: [MeetingRecordingStatus; 6] = [
    MeetingRecordingStatus::Pending,
    MeetingRecordingStatus::Joining,
    MeetingRecordingStatus::WaitingRoom,
    MeetingRecordingStatus::InMeeting,
    MeetingRecordingStatus::Recording,
    MeetingRecordingStatus::Processing,
];

/// Pauses `create_bot` until released: `reached` fires once it is waiting.
struct Hold {
    reached: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

/// A recording bot provider that reaches no one: counts the bots it is asked to send, can be
/// told to fail or to wait, and keeps the ids it is asked to stop.
#[derive(Default)]
struct CountingBot {
    created: AtomicUsize,
    fails: AtomicBool,
    hold: Mutex<Option<Hold>>,
    stopped: Mutex<Vec<String>>,
}

impl CountingBot {
    fn created(&self) -> usize {
        self.created.load(Ordering::SeqCst)
    }

    fn stopped(&self) -> Vec<String> {
        self.stopped.lock().expect("the lock is free").clone()
    }

    /// Holds the next `create_bot`; returns its `reached` signal and its release.
    fn hold_next(&self) -> (oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (reached, on_reached) = oneshot::channel();
        let (release, on_release) = oneshot::channel();
        *self.hold.lock().expect("the lock is free") = Some(Hold {
            reached,
            release: on_release,
        });
        (on_reached, release)
    }
}

#[async_trait]
impl recording_bot::Provider for CountingBot {
    async fn create_bot(&self, config: BotConfig) -> Result<Info, meeting_ai::Error> {
        let sent = self.created.fetch_add(1, Ordering::SeqCst);
        let hold = self.hold.lock().expect("the lock is free").take();
        if let Some(Hold { reached, release }) = hold {
            let _ = reached.send(());
            let _ = release.await;
        }
        if self.fails.load(Ordering::SeqCst) {
            return Err(meeting_ai::Error::Provider("the bot was refused".into()));
        }
        Ok(Info {
            id: format!("counted-bot-{sent}"),
            meeting_url: config.meeting_url,
            meeting_id: None,
            status: BotStatus::Pending,
            artifacts: None,
            error_message: None,
            status_history: vec![],
        })
    }

    async fn get_bot_status(&self, _: &str) -> Result<Info, meeting_ai::Error> {
        Err(meeting_ai::Error::Provider("unused".into()))
    }

    async fn stop_bot(&self, bot_id: &str) -> Result<(), meeting_ai::Error> {
        self.stopped
            .lock()
            .expect("the lock is free")
            .push(bot_id.to_string());
        Ok(())
    }

    async fn list_bots(&self, _: Option<Filters>) -> Result<Vec<Info>, meeting_ai::Error> {
        Ok(vec![])
    }

    fn provider_id(&self) -> &str {
        "counting_bot"
    }
}

/// Keeps every published event.
#[derive(Default)]
struct Recorder(Mutex<Vec<DomainEvent>>);

#[async_trait]
impl EventHandler for Recorder {
    async fn handle(&self, event: &DomainEvent) {
        self.0.lock().expect("the lock is free").push(event.clone());
    }
}

/// A database with the migrations' indexes and one session; returns the session.
async fn session_db() -> (DatabaseConnection, Id) {
    let db = database().await;
    mirror_migration_defaults(&db).await;
    let (session_id, _) = seed_coaching_session(&db).await;
    (db, session_id)
}

/// A session whose coach and coachee are two members of its organization; returns the
/// session and the two of them.
async fn two_member_session_db() -> (DatabaseConnection, Id, [Id; 2]) {
    let (db, session_id) = session_db().await;
    let (_, relationship) =
        entity_api::coaching_session::find_by_id_with_coaching_relationship(&db, session_id)
            .await
            .expect("the session reads");
    let (coach, coachee) = (relationship.coach_id, seed_user(&db).await);
    let organization_id = relationship.organization_id;
    let mut relationship = relationship.into_active_model();
    relationship.coachee_id = Set(coachee);
    coaching_relationships::ActiveModel::update(relationship, &db)
        .await
        .expect("the coachee is set");
    seed_membership(&db, coach, organization_id).await;
    seed_membership(&db, coachee, organization_id).await;
    (db, session_id, [coach, coachee])
}

/// Asserts `recorder` holds exactly one recording update, for `session_id`, to `participants`.
fn assert_one_update_to(recorder: &Recorder, session_id: Id, participants: [Id; 2]) {
    let events = recorder.0.lock().expect("the lock is free").clone();
    let [DomainEvent::MeetingRecordingUpdated {
        coaching_session_id,
        notify_user_ids,
    }] = events.as_slice()
    else {
        panic!("one recording update, got {events:?}");
    };
    assert_eq!(*coaching_session_id, session_id);
    let mut notified = notify_user_ids.clone();
    notified.sort();
    let mut expected = participants.to_vec();
    expected.sort();
    assert_eq!(notified, expected);
}

/// Inserts a recording of `session_id` in `status`, made `age` ago.
async fn seed_recording(
    db: &DatabaseConnection,
    session_id: Id,
    status: MeetingRecordingStatus,
    bot_id: Option<String>,
    age: TimeDelta,
) -> Id {
    let made = Utc::now() - age;
    meeting_recording::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        bot_id: Set(bot_id),
        status: Set(status),
        video_url: Set(None),
        audio_url: Set(None),
        duration_seconds: Set(None),
        started_at: Set(None),
        ended_at: Set(None),
        error_message: Set(None),
        created_at: Set(made.into()),
        updated_at: Set(made.into()),
    }
    .insert(db)
    .await
    .expect("the recording is seeded")
    .id
}

/// The session's recordings that are still active.
async fn active_recordings(db: &DatabaseConnection, session_id: Id) -> Vec<Model> {
    meeting_recording::Entity::find()
        .filter(meeting_recording::Column::CoachingSessionId.eq(session_id))
        .all(db)
        .await
        .expect("the recordings read")
        .into_iter()
        .filter(|recording| ACTIVE_STATUSES.contains(&recording.status))
        .collect()
}

async fn start_with(
    db: &DatabaseConnection,
    bot: &CountingBot,
    session_id: Id,
) -> Result<Model, Error> {
    start(
        db,
        &EventPublisher::new(),
        Some(bot),
        session_id,
        MEETING_URL,
    )
    .await
}

/// Whether the error is the 409 saying the session already has an active recording.
fn is_already_active(error: &Error) -> bool {
    matches!(
        &error.error_kind,
        DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Conflict {
            details: Some(details),
            ..
        })) if details["reason"] == reason::RECORDING_ALREADY_ACTIVE
    )
}

#[tokio::test]
async fn an_active_recording_refuses_a_start_before_any_bot_is_sent() {
    for status in ACTIVE_STATUSES {
        let (db, session_id) = session_db().await;
        let bot_id = Some(format!("bot-{}", Id::new_v4()));
        seed_recording(&db, session_id, status.clone(), bot_id, TimeDelta::zero()).await;
        let bot = CountingBot::default();

        let refused = start_with(&db, &bot, session_id)
            .await
            .expect_err("an active recording refuses the start");

        assert!(is_already_active(&refused), "{status:?}: {refused:?}");
        assert_eq!(bot.created(), 0, "{status:?}");
    }
}

#[tokio::test]
async fn the_first_start_wins_and_a_second_sends_no_bot() {
    let (db, session_id) = session_db().await;
    let bot = CountingBot::default();

    let first = start_with(&db, &bot, session_id)
        .await
        .expect("the first start succeeds");
    let second = start_with(&db, &bot, session_id)
        .await
        .expect_err("the second start is refused");

    assert!(is_already_active(&second), "{second:?}");
    let active = active_recordings(&db, session_id).await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, first.id);
    assert_eq!(bot.created(), 1);
}

#[tokio::test]
async fn a_start_racing_a_dispatching_start_is_refused_without_a_bot() {
    within_time_limit(async {
        let (db, session_id) = session_db().await;
        let bot = CountingBot::default();
        let (reached, release) = bot.hold_next();

        let (first, second) = tokio::join!(start_with(&db, &bot, session_id), async {
            reached.await.expect("the first start reaches the provider");
            let second = start_with(&db, &bot, session_id).await;
            release.send(()).expect("the first start is waiting");
            second
        });

        let first = first.expect("the first start succeeds");
        let second = second.expect_err("the racing start is refused");
        assert!(is_already_active(&second), "{second:?}");
        let active = active_recordings(&db, session_id).await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, first.id);
        assert_eq!(bot.created(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_failed_dispatch_frees_the_session_for_a_retry() {
    let (db, session_id) = session_db().await;
    let bot = CountingBot::default();
    bot.fails.store(true, Ordering::SeqCst);

    start_with(&db, &bot, session_id)
        .await
        .expect_err("the refused bot fails the start");

    let recordings = meeting_recording::Entity::find()
        .filter(meeting_recording::Column::CoachingSessionId.eq(session_id))
        .all(&db)
        .await
        .expect("the recordings read");
    let [claim] = recordings.as_slice() else {
        panic!("one claim, got {recordings:?}");
    };
    assert_eq!(claim.status, MeetingRecordingStatus::Failed);
    assert!(claim.error_message.is_some());

    bot.fails.store(false, Ordering::SeqCst);
    let retry = start_with(&db, &bot, session_id)
        .await
        .expect("the retry starts");
    assert_eq!(retry.bot_id.as_deref(), Some("counted-bot-1"));
}

#[tokio::test]
async fn an_abandoned_claim_does_not_block_a_start() {
    let (db, session_id) = session_db().await;
    let abandoned = ABANDONED_CLAIM_AFTER + TimeDelta::seconds(1);
    let old = seed_recording(
        &db,
        session_id,
        MeetingRecordingStatus::Pending,
        None,
        abandoned,
    )
    .await;
    let bot = CountingBot::default();

    let started = start_with(&db, &bot, session_id)
        .await
        .expect("the abandoned claim does not block");

    assert_ne!(started.id, old);
    let old = find_by_id(&db, old)
        .await
        .expect("the old claim reads")
        .expect("the old claim exists");
    assert_eq!(old.status, MeetingRecordingStatus::Failed);
    assert_eq!(bot.created(), 1);
}

#[tokio::test]
async fn a_young_claim_still_blocks_a_start() {
    let (db, session_id) = session_db().await;
    let young = ABANDONED_CLAIM_AFTER - TimeDelta::seconds(30);
    seed_recording(
        &db,
        session_id,
        MeetingRecordingStatus::Pending,
        None,
        young,
    )
    .await;
    let bot = CountingBot::default();

    let refused = start_with(&db, &bot, session_id)
        .await
        .expect_err("a claim still being dispatched blocks");

    assert!(is_already_active(&refused), "{refused:?}");
    assert_eq!(bot.created(), 0);
}

#[tokio::test]
async fn a_started_recording_tells_the_sessions_participants() {
    let (db, session_id) = session_db().await;
    let bot = CountingBot::default();
    let recorder = Arc::new(Recorder::default());
    let publisher = EventPublisher::new().with_handler(recorder.clone());

    start(&db, &publisher, Some(&bot), session_id, MEETING_URL)
        .await
        .expect("the start succeeds");

    let events = recorder.0.lock().expect("the lock is free").clone();
    let [DomainEvent::MeetingRecordingUpdated {
        coaching_session_id,
        ..
    }] = events.as_slice()
    else {
        panic!("one recording update, got {events:?}");
    };
    assert_eq!(*coaching_session_id, session_id);
}

#[tokio::test]
async fn a_stop_during_dispatch_recalls_the_bot_once_it_is_sent() {
    within_time_limit(async {
        let (db, session_id) = session_db().await;
        let bot = CountingBot::default();
        let (reached, release) = bot.hold_next();

        let (started, stopped) = tokio::join!(start_with(&db, &bot, session_id), async {
            reached.await.expect("the start reaches the provider");
            let stopped = stop(&db, &EventPublisher::new(), Some(&bot), session_id).await;
            release.send(()).expect("the start is waiting");
            stopped
        });

        let stopped = stopped.expect("the stop succeeds");
        let started = started.expect("the start answers with the stopped recording");
        assert_eq!(started.id, stopped.id);
        assert_eq!(started.status, MeetingRecordingStatus::Cancelled);
        assert_eq!(started.bot_id.as_deref(), Some("counted-bot-0"));
        assert_eq!(bot.stopped(), vec!["counted-bot-0".to_string()]);
        let row = find_by_id(&db, started.id)
            .await
            .expect("the recording reads")
            .expect("the recording exists");
        assert_eq!(row.status, MeetingRecordingStatus::Cancelled);
    })
    .await;
}

#[tokio::test]
async fn a_failed_dispatch_tells_both_participants() {
    let (db, session_id, participants) = two_member_session_db().await;
    let bot = CountingBot::default();
    bot.fails.store(true, Ordering::SeqCst);
    let recorder = Arc::new(Recorder::default());
    let publisher = EventPublisher::new().with_handler(recorder.clone());

    let refused = start(&db, &publisher, Some(&bot), session_id, MEETING_URL)
        .await
        .expect_err("the refused bot fails the start");

    let claim = find_latest_by_coaching_session(&db, session_id)
        .await
        .expect("the recording reads")
        .expect("the claim exists");
    assert_eq!(claim.status, MeetingRecordingStatus::Failed, "{refused:?}");
    assert_one_update_to(&recorder, session_id, participants);
}

#[tokio::test]
async fn stopping_an_active_recording_cancels_it_and_tells_both_participants() {
    let (db, session_id, participants) = two_member_session_db().await;
    let bot_id = "bot-recording".to_string();
    seed_recording(
        &db,
        session_id,
        MeetingRecordingStatus::Recording,
        Some(bot_id.clone()),
        TimeDelta::zero(),
    )
    .await;
    let bot = CountingBot::default();
    let recorder = Arc::new(Recorder::default());
    let publisher = EventPublisher::new().with_handler(recorder.clone());

    let stopped = stop(&db, &publisher, Some(&bot), session_id)
        .await
        .expect("the stop succeeds");

    assert_eq!(stopped.status, MeetingRecordingStatus::Cancelled);
    assert_eq!(bot.stopped(), vec![bot_id]);
    assert_one_update_to(&recorder, session_id, participants);
}

#[tokio::test]
async fn stopping_a_completed_recording_leaves_it_alone() {
    let (db, session_id, _) = two_member_session_db().await;
    let completed = seed_recording(
        &db,
        session_id,
        MeetingRecordingStatus::Completed,
        Some("bot-done".to_string()),
        TimeDelta::zero(),
    )
    .await;
    let bot = CountingBot::default();
    let recorder = Arc::new(Recorder::default());
    let publisher = EventPublisher::new().with_handler(recorder.clone());

    let answered = stop(&db, &publisher, Some(&bot), session_id)
        .await
        .expect("the stop answers");

    assert_eq!(answered.id, completed);
    assert_eq!(answered.status, MeetingRecordingStatus::Completed);
    let row = find_by_id(&db, completed)
        .await
        .expect("the recording reads")
        .expect("the recording exists");
    assert_eq!(row.status, MeetingRecordingStatus::Completed);
    assert!(bot.stopped().is_empty());
    assert!(recorder.0.lock().expect("the lock is free").is_empty());
}
