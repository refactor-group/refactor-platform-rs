//! Evidence gathering on completion: which outside lookups run, and who ends up attributed.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use entity::meeting_recording::{self, MeetingRecordingStatus};
use meeting_ai::types::recording::{Config as BotConfig, Filters, Info, Status as BotStatus};
use meeting_ai::types::transcription::{
    Config as TranscriptConfig, Participant as SpeakerIn, Segment as SegmentIn, Status,
    Transcription,
};
use sea_orm::sea_query::Iden;
use sea_orm::{ActiveModelTrait, DatabaseBackend, Iterable, MockDatabase, Value};

use super::*;
use crate::coaching_relationships;
use crate::oauth_connections;
use crate::transcript_participant::{MatchSource, Model as StoredParticipant};

const SESSION_MEET_URL: &str = "https://meet.google.com/abc-mnop-xyz";
const SESSION_CODE: &str = "abc-mnop-xyz";

/// Answers `get_bot_status` with a fixed meeting id (or an error, or never) and counts the calls.
struct FakeBot {
    meeting_id: Option<&'static str>,
    hangs: bool,
    calls: AtomicUsize,
}

impl FakeBot {
    fn in_meeting(meeting_id: &'static str) -> Self {
        Self {
            meeting_id: Some(meeting_id),
            hangs: false,
            calls: AtomicUsize::new(0),
        }
    }

    fn failing() -> Self {
        Self {
            meeting_id: None,
            hangs: false,
            calls: AtomicUsize::new(0),
        }
    }

    /// Sleeps ten minutes before answering in the session's space.
    fn hanging() -> Self {
        Self {
            meeting_id: Some(SESSION_CODE),
            hangs: true,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl recording_bot::Provider for FakeBot {
    async fn create_bot(&self, _: BotConfig) -> Result<Info, meeting_ai::Error> {
        Err(meeting_ai::Error::Provider("unused".into()))
    }

    async fn get_bot_status(&self, bot_id: &str) -> Result<Info, meeting_ai::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.hangs {
            tokio::time::sleep(Duration::from_secs(600)).await;
        }
        let meeting_id = self
            .meeting_id
            .ok_or_else(|| meeting_ai::Error::Network("unreachable".into()))?;
        Ok(Info {
            id: bot_id.to_string(),
            meeting_url: String::new(),
            meeting_id: Some(meeting_id.to_string()),
            status: BotStatus::Completed,
            artifacts: None,
            error_message: None,
            status_history: vec![],
        })
    }

    async fn stop_bot(&self, _: &str) -> Result<(), meeting_ai::Error> {
        Ok(())
    }

    async fn list_bots(&self, _: Option<Filters>) -> Result<Vec<Info>, meeting_ai::Error> {
        Ok(vec![])
    }

    fn provider_id(&self) -> &str {
        "fake_bot"
    }
}

/// Returns the given transcript for any id.
struct FakeTranscripts(Transcription);

#[async_trait]
impl transcription_trait::Provider for FakeTranscripts {
    async fn create_transcription(
        &self,
        _: TranscriptConfig,
    ) -> Result<Transcription, meeting_ai::Error> {
        Err(meeting_ai::Error::Provider("unused".into()))
    }

    async fn get_transcription(&self, _: &str) -> Result<Transcription, meeting_ai::Error> {
        Ok(self.0.clone())
    }

    async fn delete_transcription(&self, _: &str) -> Result<(), meeting_ai::Error> {
        Ok(())
    }

    fn provider_id(&self) -> &str {
        "fake_transcripts"
    }
}

struct People {
    coach: users::Model,
    coachee: users::Model,
    relationship: coaching_relationships::Model,
}

fn user(first: &str) -> users::Model {
    let now = Utc::now();
    users::Model {
        id: Id::new_v4(),
        email: format!("{}@test.com", first.to_lowercase()),
        first_name: first.to_owned(),
        last_name: "Test".to_owned(),
        display_name: None,
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        created_at: now.into(),
        updated_at: now.into(),
        roles: vec![],
        invite_status: None,
    }
}

fn people() -> People {
    let (coach, coachee) = (user("Jim"), user("Caleb"));
    let now = Utc::now();
    let relationship = coaching_relationships::Model {
        id: Id::new_v4(),
        organization_id: Id::new_v4(),
        coach_id: coach.id,
        coachee_id: coachee.id,
        slug: "jim-caleb".to_owned(),
        created_at: now.into(),
        updated_at: now.into(),
    };
    People {
        coach,
        coachee,
        relationship,
    }
}

fn session(people: &People, provider: Option<MeetingProvider>) -> coaching_sessions::Model {
    let now = Utc::now();
    coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id: people.relationship.id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: now.naive_utc(),
        duration_minutes: 60,
        title: None,
        meeting_url: Some(SESSION_MEET_URL.to_owned()),
        provider,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: None,
        notice_given_at: now.into(),
    }
}

fn transcription(session: &coaching_sessions::Model) -> Model {
    let now = Utc::now();
    Model {
        id: Id::new_v4(),
        coaching_session_id: session.id,
        meeting_recording_id: Id::new_v4(),
        external_id: "external-1".to_owned(),
        recall_recording_id: Some("recording-1".to_owned()),
        status: TranscriptionStatus::Processing,
        language_code: None,
        speaker_count: None,
        word_count: None,
        duration_seconds: None,
        confidence: None,
        error_message: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn recording(transcription: &Model) -> meeting_recording::Model {
    let now = Utc::now();
    meeting_recording::Model {
        id: transcription.meeting_recording_id,
        coaching_session_id: transcription.coaching_session_id,
        bot_id: "bot-1".to_owned(),
        status: MeetingRecordingStatus::Completed,
        video_url: None,
        audio_url: None,
        duration_seconds: None,
        started_at: Some(now.into()),
        ended_at: Some(now.into()),
        error_message: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// The single joined row `find_coach_and_coachee` reads, with each user's columns prefixed.
fn coach_and_coachee_row(people: &People) -> BTreeMap<String, Value> {
    [("coach_", &people.coach), ("coachee_", &people.coachee)]
        .into_iter()
        .flat_map(|(prefix, user)| {
            let user = users::ActiveModel::from(user.clone());
            users::Column::iter().filter_map(move |column| {
                user.get(column)
                    .into_value()
                    .map(|value| (format!("{prefix}{}", Iden::to_string(&column)), value))
            })
        })
        .collect()
}

fn speaker(provider_id: &str, name: &str, is_host: bool) -> SpeakerIn {
    SpeakerIn {
        provider_id: provider_id.to_owned(),
        display_name: Some(name.to_owned()),
        is_host: Some(is_host),
        platform: None,
        account_id: None,
        extra_data: None,
    }
}

/// The coach hosts as `100`; the coachee speaks as `200`.
fn speakers() -> Vec<SpeakerIn> {
    vec![speaker("100", "Jim", true), speaker("200", "Caleb", false)]
}

fn attributed(attributions: &[Attribution]) -> Vec<(&str, Option<Id>, Option<MatchSource>)> {
    attributions
        .iter()
        .map(|a| (a.provider_id.as_str(), a.user_id, a.source))
        .collect()
}

fn nobody() -> Vec<(&'static str, Option<Id>, Option<MatchSource>)> {
    vec![("100", None, None), ("200", None, None)]
}

/// Every statement the mock saw, in order, with its bound values.
fn statements(db: DatabaseConnection) -> Vec<String> {
    db.into_transaction_log()
        .iter()
        .flat_map(|transaction| {
            transaction
                .statements()
                .iter()
                .map(|statement| format!("{} {:?}", statement.sql, statement.values))
        })
        .collect()
}

#[tokio::test]
async fn a_zoom_session_attributes_nobody_without_asking_the_bot() {
    let people = people();
    let session = session(&people, Some(MeetingProvider::Zoom));
    let transcription = transcription(&session);
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![session.clone()]])
        .append_query_results([vec![coach_and_coachee_row(&people)]])
        .append_query_results([vec![recording(&transcription)]])
        .into_connection();
    let bot = FakeBot::in_meeting(SESSION_CODE);

    let attributions = attribute_speakers(
        &db,
        Some(&bot),
        &Config::default(),
        &transcription,
        &speakers(),
    )
    .await;

    assert_eq!(attributed(&attributions), nobody());
    assert_eq!(bot.calls(), 0);
}

#[tokio::test]
async fn a_bot_in_another_meeting_attributes_nobody_and_never_reaches_google() {
    let people = people();
    let session = session(&people, Some(MeetingProvider::Google));
    let transcription = transcription(&session);
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![session.clone()]])
        .append_query_results([vec![coach_and_coachee_row(&people)]])
        .append_query_results([vec![recording(&transcription)]])
        .into_connection();
    let bot = FakeBot::in_meeting("zzz-zzzz-zzz");

    let attributions = attribute_speakers(
        &db,
        Some(&bot),
        &Config::default(),
        &transcription,
        &speakers(),
    )
    .await;

    assert_eq!(attributed(&attributions), nobody());
    assert_eq!(bot.calls(), 1);
    assert!(statements(db)
        .iter()
        .all(|sql| !sql.contains("oauth_connections")));
}

/// Asserted on the attribution handed to `persist_completion`, the direct input to the rows.
#[tokio::test]
async fn the_session_space_with_a_host_attributes_the_coach_and_the_coachee() {
    let people = people();
    let session = session(&people, Some(MeetingProvider::Google));
    let transcription = transcription(&session);
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![session.clone()]])
        .append_query_results([vec![coach_and_coachee_row(&people)]])
        .append_query_results([vec![recording(&transcription)]])
        .append_query_results([Vec::<oauth_connections::Model>::new()])
        .into_connection();
    let bot = FakeBot::in_meeting(SESSION_CODE);

    let attributions = attribute_speakers(
        &db,
        Some(&bot),
        &Config::default(),
        &transcription,
        &speakers(),
    )
    .await;

    assert_eq!(
        attributed(&attributions),
        vec![
            ("100", Some(people.coach.id), Some(MatchSource::Account)),
            (
                "200",
                Some(people.coachee.id),
                Some(MatchSource::Elimination)
            ),
        ]
    );
    assert!(statements(db)
        .iter()
        .any(|sql| sql.contains("oauth_connections")));
}

/// Runs `handle_completion` against a Google session with the given bot; returns its result,
/// the statement log, and the session's people.
async fn complete_with(bot: &FakeBot) -> (Result<(), Error>, Vec<String>, People) {
    let people = people();
    let session = session(&people, Some(MeetingProvider::Google));
    let transcription = transcription(&session);
    let completed = Model {
        status: TranscriptionStatus::Completed,
        ..transcription.clone()
    };
    let stored = |provider_id: &str| StoredParticipant {
        id: Id::new_v4(),
        transcription_id: transcription.id,
        provider_participant_id: provider_id.to_owned(),
        display_name: None,
        is_host: None,
        platform: None,
        platform_account_id: None,
        extra_data: None,
        user_id: None,
        match_source: None,
        created_at: Utc::now().into(),
    };
    let segment = |provider_id: &str, text: &str| SegmentIn {
        text: text.to_owned(),
        speaker: provider_id.to_owned(),
        participant_id: Some(provider_id.to_owned()),
        start_ms: 0,
        end_ms: 1000,
        confidence: 0.0,
        words: vec![],
    };
    let result = Transcription {
        id: "external-1".to_owned(),
        status: Status::Completed,
        text: None,
        words: vec![],
        segments: vec![segment("100", "Good morning."), segment("200", "Morning.")],
        participants: speakers(),
        chapters: vec![],
        sentiment_analysis: vec![],
        confidence: None,
        duration_seconds: None,
        language_code: None,
        speaker_count: None,
        error_message: None,
    };
    let stored_segment = |text: &str| entity::transcript_segment::Model {
        id: Id::new_v4(),
        transcription_id: transcription.id,
        participant_id: None,
        speaker_label: "x".to_owned(),
        text: text.to_owned(),
        start_ms: 0,
        end_ms: 1000,
        confidence: None,
        sentiment: None,
        created_at: Utc::now().into(),
    };
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![transcription.clone()]])
        .append_query_results([vec![session.clone()]])
        .append_query_results([vec![coach_and_coachee_row(&people)]])
        .append_query_results([vec![recording(&transcription)]])
        .append_query_results([vec![stored("100"), stored("200")]])
        .append_query_results([vec![
            stored_segment("Good morning."),
            stored_segment("Morning."),
        ]])
        .append_query_results([vec![transcription.clone()]])
        .append_query_results([vec![completed]])
        .into_connection();

    let outcome = handle_completion(
        &db,
        Some(&FakeTranscripts(result)),
        Some(bot),
        &Config::default(),
        "external-1",
    )
    .await;

    (outcome, statements(db), people)
}

/// The transcript completes, its segments are stored, and nobody is attributed.
fn assert_completed_with_nobody_attributed(log: &[String], people: &People) {
    assert!(log
        .iter()
        .any(|sql| sql.contains(r#"INSERT INTO "refactor_platform"."transcript_segments""#)));
    let completion = log
        .iter()
        .find(|sql| sql.starts_with(r#"UPDATE "refactor_platform"."transcriptions""#))
        .expect("the transcription is updated");
    assert!(completion.contains("completed"));
    let people_ids = [people.coach.id.to_string(), people.coachee.id.to_string()];
    assert!(log
        .iter()
        .filter(|sql| sql.contains("transcript_participants"))
        .all(|sql| people_ids.iter().all(|id| !sql.contains(id.as_str()))));
}

#[tokio::test]
async fn an_unreadable_bot_still_completes_the_transcript_with_nobody_attributed() {
    let bot = FakeBot::failing();

    let (outcome, log, people) = complete_with(&bot).await;

    outcome.expect("the transcript completes");
    assert_eq!(bot.calls(), 1);
    assert_completed_with_nobody_attributed(&log, &people);
}

#[tokio::test(start_paused = true)]
async fn a_hung_bot_still_completes_the_transcript_with_nobody_attributed() {
    let bot = FakeBot::hanging();

    let (outcome, log, people) =
        tokio::time::timeout(Duration::from_secs(300), complete_with(&bot))
            .await
            .expect("completion must not wait on a hung bot");

    outcome.expect("the transcript completes");
    assert_eq!(bot.calls(), 1);
    assert_completed_with_nobody_attributed(&log, &people);
}
