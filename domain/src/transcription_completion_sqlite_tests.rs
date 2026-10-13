//! Persisting a completed transcript: participants with their attribution, and segments linked
//! to the participant who spoke them, checked against a real SQL engine.

use chrono::Utc;
use entity::meeting_recording::{self, MeetingRecordingStatus};
use entity::{coaching_relationships, users};
use meeting_ai::types::transcription::{
    Participant as SpeakerIn, Segment as SegmentIn, Status, Transcription,
};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};
use serde_json::json;

use super::*;
use crate::error::Error;
use crate::test_utils::sqlite::{database, seed_organization, within_time_limit};
use crate::transcript_attribution::Attribution;
use crate::transcript_participant::{self as participant_api, MatchSource};

async fn seed_user_named(db: &DatabaseConnection, display_name: &str) -> users::Model {
    let id = Id::new_v4();
    let now = Utc::now();
    users::ActiveModel {
        id: Set(id),
        email: Set(format!("{id}@example.com")),
        first_name: Set("Test".to_string()),
        last_name: Set("User".to_string()),
        display_name: Set(Some(display_name.to_string())),
        password: Set(None),
        github_username: Set(None),
        github_profile_url: Set(None),
        timezone: Set("UTC".to_string()),
        default_coaching_session_duration_minutes: Set(60),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the user is seeded")
}

struct Fixture {
    db: DatabaseConnection,
    session: coaching_sessions::Model,
    coach: users::Model,
    coachee: users::Model,
    transcription: Model,
}

async fn fixture() -> Fixture {
    let db = database().await;
    let coach = seed_user_named(&db, "Coach Profile").await;
    let coachee = seed_user_named(&db, "Coachee Profile").await;
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
        meeting_url: Set(Some("https://meet.google.com/abc-mnop-xyz".to_string())),
        provider: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        hydrated_at: Set(None),
        notice_given_at: Set(now.into()),
    }
    .insert(&db)
    .await
    .expect("the session is seeded");

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

    Fixture {
        db,
        session,
        coach,
        coachee,
        transcription,
    }
}

fn speaker(
    provider_id: &str,
    name: Option<&str>,
    is_host: bool,
    account: Option<&str>,
) -> SpeakerIn {
    SpeakerIn {
        provider_id: provider_id.to_string(),
        display_name: name.map(str::to_string),
        is_host: Some(is_host),
        platform: Some("desktop".to_string()),
        account_id: account.map(str::to_string),
        extra_data: Some(json!({ "google_meet": { "static_participant_id": provider_id } })),
    }
}

fn segment(participant_id: Option<&str>, label: &str, text: &str, start_ms: i64) -> SegmentIn {
    SegmentIn {
        text: text.to_string(),
        speaker: label.to_string(),
        participant_id: participant_id.map(str::to_string),
        start_ms,
        end_ms: start_ms + 1000,
        confidence: 0.0,
        words: vec![],
    }
}

fn transcript(participants: Vec<SpeakerIn>, segments: Vec<SegmentIn>) -> Transcription {
    Transcription {
        id: "ext-1".to_string(),
        status: Status::Completed,
        text: None,
        words: vec![],
        segments,
        participants,
        chapters: vec![],
        sentiment_analysis: vec![],
        confidence: None,
        duration_seconds: None,
        language_code: None,
        speaker_count: None,
        error_message: None,
    }
}

fn attributed(provider_id: &str, user: Option<(Id, MatchSource)>) -> Attribution {
    Attribution {
        provider_id: provider_id.to_string(),
        user_id: user.map(|(id, _)| id),
        source: user.map(|(_, source)| source),
    }
}

/// Coach host `100`, coachee `200`, nameless `300`, and a line no participant claims.
fn completed(f: &Fixture) -> (Transcription, Vec<Attribution>) {
    (
        transcript(
            vec![
                speaker("100", Some("Coach Typed"), true, None),
                speaker("200", Some("Coachee Typed"), false, Some("acct-200")),
                speaker("300", None, false, None),
            ],
            vec![
                segment(Some("100"), "Coach Typed", "Good morning.", 0),
                segment(Some("200"), "Coachee Typed", "Morning.", 4000),
                segment(Some("300"), "300", "Hello?", 9000),
                segment(None, "Unknown", "Orphan line.", 12000),
            ],
        ),
        vec![
            attributed("100", Some((f.coach.id, MatchSource::Account))),
            attributed("200", Some((f.coachee.id, MatchSource::Elimination))),
            attributed("300", None),
        ],
    )
}

#[tokio::test]
async fn participants_are_stored_with_raw_data_and_attribution() -> Result<(), Error> {
    within_time_limit(async {
        let f = fixture().await;
        let (result, attributions) = completed(&f);

        persist_completion(&f.db, &f.transcription, &result, &attributions).await?;

        let mut stored = participant_api::find_by_transcription(&f.db, f.transcription.id).await?;
        stored.sort_by(|a, b| a.provider_participant_id.cmp(&b.provider_participant_id));
        let summary: Vec<_> = stored
            .iter()
            .map(|p| {
                (
                    p.provider_participant_id.as_str(),
                    p.display_name.as_deref(),
                    p.is_host,
                    p.platform_account_id.as_deref(),
                    p.user_id,
                    p.match_source,
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "100",
                    Some("Coach Typed"),
                    Some(true),
                    None,
                    Some(f.coach.id),
                    Some(MatchSource::Account)
                ),
                (
                    "200",
                    Some("Coachee Typed"),
                    Some(false),
                    Some("acct-200"),
                    Some(f.coachee.id),
                    Some(MatchSource::Elimination)
                ),
                ("300", None, Some(false), None, None, None),
            ]
        );
        assert_eq!(
            stored[0].extra_data,
            Some(json!({ "google_meet": { "static_participant_id": "100" } }))
        );
        assert_eq!(stored[0].platform.as_deref(), Some("desktop"));

        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn segments_link_to_the_participant_who_spoke_them() -> Result<(), Error> {
    within_time_limit(async {
        let f = fixture().await;
        let (result, attributions) = completed(&f);

        persist_completion(&f.db, &f.transcription, &result, &attributions).await?;

        let participants =
            participant_api::find_by_transcription(&f.db, f.transcription.id).await?;
        let row_of = |provider: &str| {
            participants
                .iter()
                .find(|p| p.provider_participant_id == provider)
                .map(|p| p.id)
        };
        let segments =
            entity_api::transcript_segment::find_by_transcription(&f.db, f.transcription.id)
                .await?;
        let links: Vec<_> = segments
            .iter()
            .map(|s| (s.speaker_label.as_str(), s.text.as_str(), s.participant_id))
            .collect();

        assert_eq!(
            links,
            vec![
                ("Coach Typed", "Good morning.", row_of("100")),
                ("Coachee Typed", "Morning.", row_of("200")),
                ("300", "Hello?", row_of("300")),
                ("Unknown", "Orphan line.", None),
            ]
        );

        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn a_segment_naming_an_unknown_participant_is_kept_unlinked() -> Result<(), Error> {
    within_time_limit(async {
        let f = fixture().await;
        let result = transcript(
            vec![speaker("100", Some("Coach Typed"), true, None)],
            vec![segment(Some("999"), "Ghost", "Who am I?", 0)],
        );

        persist_completion(&f.db, &f.transcription, &result, &[attributed("100", None)]).await?;

        let segments =
            entity_api::transcript_segment::find_by_transcription(&f.db, f.transcription.id)
                .await?;
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].participant_id, None);

        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn an_empty_transcript_writes_nothing() -> Result<(), Error> {
    within_time_limit(async {
        let f = fixture().await;

        persist_completion(&f.db, &f.transcription, &transcript(vec![], vec![]), &[]).await?;

        assert!(
            participant_api::find_by_transcription(&f.db, f.transcription.id)
                .await?
                .is_empty()
        );
        assert!(
            entity_api::transcript_segment::find_by_transcription(&f.db, f.transcription.id)
                .await?
                .is_empty()
        );

        Ok::<(), Error>(())
    })
    .await
}

/// End to end on the read side: stored attribution drives labels, roles, and users.
#[tokio::test]
async fn persisted_attribution_reads_back_as_profile_names_and_roles() -> Result<(), Error> {
    within_time_limit(async {
        let f = fixture().await;
        let (result, attributions) = completed(&f);

        persist_completion(&f.db, &f.transcription, &result, &attributions).await?;
        let read = read_segments(&f.db, &f.session, f.transcription.id).await?;

        let labeled: Vec<_> = read
            .iter()
            .map(|s| (s.speaker_label.as_str(), s.speaker_user_id, s.speaker_role))
            .collect();
        assert_eq!(
            labeled,
            vec![
                ("Coach Profile", Some(f.coach.id), Some(SpeakerRole::Coach)),
                (
                    "Coachee Profile",
                    Some(f.coachee.id),
                    Some(SpeakerRole::Coachee)
                ),
                ("Guest 1", None, None),
                ("Unknown", None, None),
            ]
        );

        Ok::<(), Error>(())
    })
    .await
}

async fn stored_transcription(f: &Fixture) -> Result<Model, Error> {
    Ok(
        entity_api::transcription::find_by_external_id(&f.db, &f.transcription.external_id)
            .await?
            .expect("the transcription exists"),
    )
}

#[tokio::test]
async fn completion_is_recorded_with_the_rows() -> Result<(), Error> {
    within_time_limit(async {
        let f = fixture().await;
        let (result, attributions) = completed(&f);

        persist_completion(&f.db, &f.transcription, &result, &attributions).await?;

        let stored = stored_transcription(&f).await?;
        assert_eq!(stored.status, TranscriptionStatus::Completed);
        assert_eq!(stored.word_count, Some(6));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn an_empty_transcript_is_still_completed() -> Result<(), Error> {
    within_time_limit(async {
        let f = fixture().await;

        persist_completion(&f.db, &f.transcription, &transcript(vec![], vec![]), &[]).await?;

        let stored = stored_transcription(&f).await?;
        assert_eq!(stored.status, TranscriptionStatus::Completed);
        assert_eq!(stored.word_count, Some(0));
        Ok(())
    })
    .await
}
