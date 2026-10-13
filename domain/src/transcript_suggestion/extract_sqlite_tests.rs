//! One extraction run against a real SQL engine with a fake model: what it stores, what it
//! completes, and what it announces.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use entity::meeting_recording::{self, MeetingRecordingStatus};
use entity::transcription::TranscriptionStatus;
use entity::{actions, coaching_relationships, coaching_sessions, transcript_analysis, users};
use meeting_ai::types::analysis::{Completion, Request, Usage};
use meeting_ai::types::transcription::{
    Participant as SpeakerIn, Segment as SegmentIn, Status as TranscriptStatus, Transcription,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde_json::json;

use super::*;
use crate::events::EventHandler;
use crate::status::Status as ActionStatus;
use crate::test_utils::sqlite::{database, seed_organization, within_time_limit};
use crate::transcript_analysis::{claim, create_pending, Status as AnalysisStatus};
use crate::transcript_attribution::Attribution;
use crate::transcript_participant::MatchSource;
use crate::transcript_suggestion::{ActionProposal, Actions, Model, Target};
use crate::transcription::persist_completion;

const COACH_NAME: &str = "Jordan Coach";

/// Answers every call with the same output and keeps each request it was sent.
struct FakeModel {
    output: serde_json::Value,
    requests: Mutex<Vec<Request>>,
}

impl FakeModel {
    fn answering(output: serde_json::Value) -> Self {
        Self {
            output,
            requests: Mutex::new(vec![]),
        }
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().expect("the lock is free").clone()
    }
}

#[async_trait]
impl analysis::Provider for FakeModel {
    async fn complete(&self, request: Request) -> Result<Completion, meeting_ai::Error> {
        self.requests
            .lock()
            .expect("the lock is free")
            .push(request);
        Ok(Completion {
            output: self.output.clone(),
            model: "fake-model".to_string(),
            usage: Usage {
                input_tokens: 1200,
                output_tokens: 300,
            },
            request_id: None,
        })
    }

    fn provider_id(&self) -> &str {
        "fake"
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

fn recording_publisher() -> (EventPublisher, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    (
        EventPublisher::new().with_handler(recorder.clone()),
        recorder,
    )
}

fn suggestion_changed_events(recorder: &Recorder) -> Vec<DomainEvent> {
    recorder
        .0
        .lock()
        .expect("the lock is free")
        .iter()
        .filter(|e| matches!(e, DomainEvent::TranscriptSuggestionsChanged { .. }))
        .cloned()
        .collect()
}

struct Fixture {
    db: DatabaseConnection,
    session_id: Id,
    coachee_id: Id,
    transcription: crate::transcription::Model,
    action_id: Id,
}

async fn seed_user_named(db: &DatabaseConnection, name: &str) -> users::Model {
    let id = Id::new_v4();
    let now = Utc::now();
    users::ActiveModel {
        id: Set(id),
        email: Set(format!("{id}@example.com")),
        first_name: Set("Test".to_string()),
        last_name: Set("User".to_string()),
        display_name: Set(Some(name.to_string())),
        password: Set(None),
        github_username: Set(None),
        github_profile_url: Set(None),
        timezone: Set("America/Chicago".to_string()),
        default_coaching_session_duration_minutes: Set(60),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the user is seeded")
}

/// A coach and coachee, their session with one action, and a transcription still processing.
async fn fixture() -> Fixture {
    let db = database().await;
    let coach = seed_user_named(&db, COACH_NAME).await;
    let coachee = seed_user_named(&db, "Casey Coachee").await;
    let now = Utc::now();

    let relationship_id = Id::new_v4();
    coaching_relationships::ActiveModel {
        id: Set(relationship_id),
        organization_id: Set(seed_organization(&db).await),
        coach_id: Set(coach.id),
        coachee_id: Set(coachee.id),
        slug: Set(format!("relationship-{relationship_id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(&db)
    .await
    .expect("the relationship is seeded");

    let session = coaching_sessions::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_relationship_id: Set(relationship_id),
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
    .insert(&db)
    .await
    .expect("the session is seeded");

    let action = actions::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session.id),
        goal_id: Set(None),
        user_id: Set(coach.id),
        body: Set(Some("Share the org chart".to_string())),
        due_by: Set(None),
        status: Set(ActionStatus::NotStarted),
        status_changed_at: Set(now.into()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(&db)
    .await
    .expect("the action is seeded");

    let recording_id = Id::new_v4();
    meeting_recording::ActiveModel {
        id: Set(recording_id),
        coaching_session_id: Set(session.id),
        bot_id: Set("bot-1".to_string()),
        status: Set(MeetingRecordingStatus::Completed),
        video_url: Set(None),
        audio_url: Set(None),
        duration_seconds: Set(None),
        started_at: Set(None),
        ended_at: Set(None),
        error_message: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(&db)
    .await
    .expect("the recording is seeded");

    let transcription = entity::transcription::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session.id),
        meeting_recording_id: Set(recording_id),
        external_id: Set("ext-1".to_string()),
        recall_recording_id: Set(None),
        status: Set(TranscriptionStatus::Processing),
        language_code: Set(None),
        speaker_count: Set(None),
        word_count: Set(None),
        duration_seconds: Set(None),
        confidence: Set(None),
        error_message: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(&db)
    .await
    .expect("the transcription is seeded");

    let fixture = Fixture {
        db,
        session_id: session.id,
        coachee_id: coachee.id,
        transcription,
        action_id: action.id,
    };
    let (coach_id, coachee_id) = (coach.id, coachee.id);
    complete_transcript(&fixture, coach_id, coachee_id).await;
    fixture
}

fn speaker(provider_id: &str) -> SpeakerIn {
    SpeakerIn {
        provider_id: provider_id.to_string(),
        display_name: None,
        is_host: Some(false),
        platform: None,
        account_id: None,
        extra_data: None,
    }
}

fn segment(provider_id: &str, text: &str, start_ms: i64) -> SegmentIn {
    SegmentIn {
        text: text.to_string(),
        speaker: provider_id.to_string(),
        participant_id: Some(provider_id.to_string()),
        start_ms,
        end_ms: start_ms + 1000,
        confidence: 0.0,
        words: vec![],
    }
}

/// Three segments: coach, coachee, coach.
async fn complete_transcript(f: &Fixture, coach_id: Id, coachee_id: Id) {
    let result = Transcription {
        id: "ext-1".to_string(),
        status: TranscriptStatus::Completed,
        text: None,
        words: vec![],
        segments: vec![
            segment("100", "What will you do before we next meet?", 0),
            segment("200", "I will send you the org chart by Friday.", 4000),
            segment("100", "Great, thanks.", 9000),
        ],
        participants: vec![speaker("100"), speaker("200")],
        chapters: vec![],
        sentiment_analysis: vec![],
        confidence: None,
        duration_seconds: None,
        language_code: None,
        speaker_count: None,
        error_message: None,
    };
    let attributions =
        [("100", coach_id), ("200", coachee_id)].map(|(provider_id, user_id)| Attribution {
            provider_id: provider_id.to_string(),
            user_id: Some(user_id),
            source: Some(MatchSource::Account),
        });
    persist_completion(&f.db, &f.transcription, &result, &attributions)
        .await
        .expect("the transcript is completed");
}

/// Segment ids in speaking order.
async fn segment_ids(f: &Fixture) -> Vec<Id> {
    entity::transcript_segment::Entity::find()
        .filter(entity::transcript_segment::Column::TranscriptionId.eq(f.transcription.id))
        .order_by_asc(entity::transcript_segment::Column::StartMs)
        .all(&f.db)
        .await
        .expect("the segments are read")
        .into_iter()
        .map(|s| s.id)
        .collect()
}

async fn claimed_analysis(
    db: &DatabaseConnection,
    transcription_id: Id,
) -> transcript_analysis::Model {
    let pending = create_pending(db, transcription_id, Target::Action)
        .await
        .expect("the analysis is created");
    let now = Utc::now().fixed_offset();
    claim(db, pending.id, now, now - Duration::minutes(10), 3)
        .await
        .expect("the claim runs")
        .expect("the analysis is claimed")
}

async fn reread_analysis(db: &DatabaseConnection, id: Id) -> transcript_analysis::Model {
    transcript_analysis::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the analysis is read")
        .expect("the analysis exists")
}

async fn stored_suggestions(db: &DatabaseConnection) -> Vec<Model> {
    entity::transcript_suggestion::Entity::find()
        .all(db)
        .await
        .expect("the suggestions are read")
}

fn candidate(source_segment: &str, body: &str, duplicate_of: Option<&str>) -> serde_json::Value {
    json!({
        "body": body,
        "assignees": ["coachee"],
        "due_date": null,
        "due_phrase": null,
        "source_segment": source_segment,
        "quote": "send you the org chart",
        "duplicate_of": duplicate_of,
    })
}

fn two_suggestions() -> serde_json::Value {
    json!({ "suggestions": [
        candidate("s2", "Send the org chart to the coach", Some("a1")),
        candidate("s9", "Book a follow-up", None),
    ]})
}

#[tokio::test]
async fn a_run_stores_the_verified_suggestion_and_completes_the_analysis() {
    within_time_limit(async {
        let f = fixture().await;
        let analysis = claimed_analysis(&f.db, f.transcription.id).await;
        let model = FakeModel::answering(two_suggestions());
        let (publisher, _) = recording_publisher();

        let outcome = run::<Actions>(&f.db, &model, &publisher, &analysis)
            .await
            .expect("the run succeeds");
        assert_eq!(outcome, Outcome::Stored(1));

        let rows = stored_suggestions(&f.db).await;
        assert_eq!(rows.len(), 1, "the unknown s9 is dropped");
        let row = &rows[0];
        assert_eq!(row.coaching_session_id, f.session_id);
        assert_eq!(row.transcript_analysis_id, analysis.id);
        assert_eq!(row.target, Target::Action);
        assert_eq!(row.segment_id, segment_ids(&f).await[1]);
        assert_eq!(row.duplicate_of_id, Some(f.action_id));
        assert_eq!(row.status, Status::Pending);
        let proposal: ActionProposal =
            serde_json::from_value(row.proposal.clone()).expect("the proposal deserializes");
        assert_eq!(
            proposal.action.action.body.as_deref(),
            Some("Send the org chart to the coach")
        );
        assert_eq!(proposal.action.assignee_ids, vec![f.coachee_id]);

        let stored = reread_analysis(&f.db, analysis.id).await;
        assert_eq!(stored.status, AnalysisStatus::Completed);
        assert_eq!(stored.model.as_deref(), Some("fake-model"));
        assert_eq!(stored.input_tokens, Some(1200));
        assert_eq!(stored.output_tokens, Some(300));

        let requests = model.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].content.contains("[s1 "));
        assert!(requests[0].content.contains(COACH_NAME));
    })
    .await;
}

#[tokio::test]
async fn a_run_whose_claim_was_retaken_stores_nothing() {
    within_time_limit(async {
        let f = fixture().await;
        let analysis = claimed_analysis(&f.db, f.transcription.id).await;
        // Another worker's re-claim bumps the attempts.
        transcript_analysis::ActiveModel {
            id: Set(analysis.id),
            attempts: Set(analysis.attempts + 1),
            ..Default::default()
        }
        .update(&f.db)
        .await
        .expect("the re-claim is simulated");
        let model = FakeModel::answering(two_suggestions());
        let (publisher, recorder) = recording_publisher();

        let outcome = run::<Actions>(&f.db, &model, &publisher, &analysis)
            .await
            .expect("the run succeeds");

        assert_eq!(outcome, Outcome::Superseded);
        assert!(stored_suggestions(&f.db).await.is_empty());
        assert_eq!(
            reread_analysis(&f.db, analysis.id).await.status,
            AnalysisStatus::Processing
        );
        assert!(suggestion_changed_events(&recorder).is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_transcript_without_segments_completes_without_calling_the_model() {
    within_time_limit(async {
        let db = database().await;
        let seeded = crate::test_utils::sqlite::seed_transcription(&db, 0).await;
        let analysis = claimed_analysis(&db, seeded.transcription_id).await;
        let model = FakeModel::answering(two_suggestions());
        let (publisher, recorder) = recording_publisher();

        let outcome = run::<Actions>(&db, &model, &publisher, &analysis)
            .await
            .expect("the run succeeds");

        assert_eq!(outcome, Outcome::Stored(0));
        assert!(model.requests().is_empty());
        assert!(stored_suggestions(&db).await.is_empty());
        let stored = reread_analysis(&db, analysis.id).await;
        assert_eq!(stored.status, AnalysisStatus::Completed);
        assert_eq!(stored.input_tokens, None);
        assert_eq!(suggestion_changed_events(&recorder).len(), 1);
    })
    .await;
}

#[tokio::test]
async fn every_completed_run_announces_once_with_or_without_suggestions() {
    within_time_limit(async {
        let f = fixture().await;
        let analysis = claimed_analysis(&f.db, f.transcription.id).await;
        let (publisher, recorder) = recording_publisher();

        run::<Actions>(
            &f.db,
            &FakeModel::answering(two_suggestions()),
            &publisher,
            &analysis,
        )
        .await
        .expect("the run succeeds");

        let events = suggestion_changed_events(&recorder);
        assert_eq!(events.len(), 1);
        let DomainEvent::TranscriptSuggestionsChanged {
            coaching_session_id,
            target,
            ..
        } = &events[0]
        else {
            unreachable!("filtered to this variant");
        };
        assert_eq!(*coaching_session_id, f.session_id);
        assert_eq!(target, "action");

        let other = fixture().await;
        let analysis = claimed_analysis(&other.db, other.transcription.id).await;
        let (publisher, recorder) = recording_publisher();
        let nothing_valid = json!({ "suggestions": [candidate("s9", "Book a follow-up", None)] });

        let outcome = run::<Actions>(
            &other.db,
            &FakeModel::answering(nothing_valid),
            &publisher,
            &analysis,
        )
        .await
        .expect("the run succeeds");

        assert_eq!(outcome, Outcome::Stored(0));
        assert_eq!(suggestion_changed_events(&recorder).len(), 1);
    })
    .await;
}
