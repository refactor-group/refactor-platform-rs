//! Transcript reads against a real SQL engine: labels and roles come from the stored
//! participants of the right transcription, and another session's rows never leak in.

use chrono::Utc;
use entity::meeting_recording::{self, MeetingRecordingStatus};
use entity::{coaching_relationships, transcript_participant, transcript_segment, users};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};

use super::*;
use crate::error::Error;
use crate::test_utils::sqlite::{database, seed_organization, within_time_limit};
use crate::transcript_participant::MatchSource;

/// Inserts a user with every column set and the given display name.
async fn seed_named_user(db: &DatabaseConnection, display_name: &str) -> users::Model {
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

/// Inserts a session whose relationship pairs the two given users.
async fn seed_session(
    db: &DatabaseConnection,
    coach_id: Id,
    coachee_id: Id,
) -> coaching_sessions::Model {
    let organization_id = seed_organization(db).await;
    let now = Utc::now();
    let relationship_id = Id::new_v4();

    coaching_relationships::ActiveModel {
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
    .expect("the coaching relationship is seeded");

    coaching_sessions::ActiveModel {
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
    .insert(db)
    .await
    .expect("the coaching session is seeded")
}

/// Records a completed transcription for the session and returns its id.
async fn seed_transcription(db: &DatabaseConnection, coaching_session_id: Id) -> Id {
    let now = Utc::now();
    let recording_id = Id::new_v4();

    meeting_recording::ActiveModel {
        id: Set(recording_id),
        coaching_session_id: Set(coaching_session_id),
        bot_id: Set(format!("bot-{recording_id}")),
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
    .insert(db)
    .await
    .expect("the recording is seeded");

    let id = Id::new_v4();
    entity::transcription::ActiveModel {
        id: Set(id),
        coaching_session_id: Set(coaching_session_id),
        meeting_recording_id: Set(recording_id),
        external_id: Set(format!("ext-{id}")),
        recall_recording_id: Set(None),
        status: Set(TranscriptionStatus::Completed),
        language_code: Set(None),
        speaker_count: Set(None),
        word_count: Set(None),
        duration_seconds: Set(None),
        confidence: Set(None),
        error_message: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the transcription is seeded");

    id
}

/// Inserts a participant with every column set and returns its id.
async fn seed_participant(
    db: &DatabaseConnection,
    transcription_id: Id,
    display_name: Option<&str>,
    attributed_to: Option<(Id, MatchSource)>,
) -> Id {
    let id = Id::new_v4();
    transcript_participant::ActiveModel {
        id: Set(id),
        transcription_id: Set(transcription_id),
        provider_participant_id: Set(id.to_string()),
        display_name: Set(display_name.map(str::to_owned)),
        is_host: Set(None),
        platform: Set(None),
        platform_account_id: Set(None),
        extra_data: Set(None),
        user_id: Set(attributed_to.map(|(user_id, _)| user_id)),
        match_source: Set(attributed_to.map(|(_, source)| source)),
        created_at: Set(Utc::now().into()),
    }
    .insert(db)
    .await
    .expect("the participant is seeded");
    id
}

/// Inserts a segment spoken by the participant, with a raw provider label.
async fn seed_segment(
    db: &DatabaseConnection,
    transcription_id: Id,
    participant_id: Id,
    raw_label: &str,
    start_ms: i32,
) {
    transcript_segment::ActiveModel {
        id: Set(Id::new_v4()),
        transcription_id: Set(transcription_id),
        participant_id: Set(Some(participant_id)),
        speaker_label: Set(raw_label.to_string()),
        text: Set(format!("Line at {start_ms}.")),
        start_ms: Set(start_ms),
        end_ms: Set(start_ms + 1000),
        confidence: Set(None),
        sentiment: Set(None),
        created_at: Set(Utc::now().into()),
    }
    .insert(db)
    .await
    .expect("the segment is seeded");
}

struct Fixture {
    db: DatabaseConnection,
    session: coaching_sessions::Model,
    coach: users::Model,
    coachee: users::Model,
    transcription_id: Id,
    other_transcription_id: Id,
}

/// Coach `Jim H` and coachee `cbourg2`, a four-participant transcript, and another
/// session's transcript with its own participants.
async fn fixture() -> Fixture {
    let db = database().await;
    let coach = seed_named_user(&db, "Jim H").await;
    let coachee = seed_named_user(&db, "cbourg2").await;
    let session = seed_session(&db, coach.id, coachee.id).await;
    let transcription_id = seed_transcription(&db, session.id).await;

    let coach_p = seed_participant(
        &db,
        transcription_id,
        Some("J. Hodapp"),
        Some((coach.id, MatchSource::Account)),
    )
    .await;
    let coachee_p = seed_participant(
        &db,
        transcription_id,
        Some("CB"),
        Some((coachee.id, MatchSource::Elimination)),
    )
    .await;
    let typed_jim = seed_participant(&db, transcription_id, Some("Jim H"), None).await;
    let nameless = seed_participant(&db, transcription_id, None, None).await;

    seed_segment(&db, transcription_id, coach_p, "J. Hodapp", 0).await;
    seed_segment(&db, transcription_id, coachee_p, "CB", 4000).await;
    seed_segment(&db, transcription_id, typed_jim, "Jim H", 9000).await;
    seed_segment(&db, transcription_id, nameless, "400", 20000).await;
    seed_segment(&db, transcription_id, coachee_p, "CB", 59000).await;

    let other_session = seed_session(&db, coach.id, coachee.id).await;
    let other_transcription_id = seed_transcription(&db, other_session.id).await;
    let intruder = seed_participant(&db, other_transcription_id, Some("Intruder"), None).await;
    seed_segment(&db, other_transcription_id, intruder, "Intruder", 1000).await;

    Fixture {
        db,
        session,
        coach,
        coachee,
        transcription_id,
        other_transcription_id,
    }
}

#[tokio::test]
async fn speakers_are_labeled_from_stored_attribution() -> Result<(), Error> {
    within_time_limit(async {
        let f = fixture().await;

        let read = read_with_speakers(&f.db, &f.session, f.transcription_id).await?;

        let speakers: Vec<(&str, Option<SpeakerRole>)> = read
            .speakers
            .iter()
            .map(|speaker| (speaker.label.as_str(), speaker.role))
            .collect();
        assert_eq!(
            speakers,
            vec![
                ("Jim H", Some(SpeakerRole::Coach)),
                ("cbourg2", Some(SpeakerRole::Coachee)),
                ("Jim H (2)", None),
                ("Guest 1", None),
            ]
        );

        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn segments_carry_the_same_labels_and_attribution() -> Result<(), Error> {
    within_time_limit(async {
        let f = fixture().await;

        let segments = read_segments(&f.db, &f.session, f.transcription_id).await?;

        let read: Vec<(&str, Option<Id>, Option<SpeakerRole>)> = segments
            .iter()
            .map(|s| (s.speaker_label.as_str(), s.speaker_user_id, s.speaker_role))
            .collect();
        assert_eq!(
            read,
            vec![
                ("Jim H", Some(f.coach.id), Some(SpeakerRole::Coach)),
                ("cbourg2", Some(f.coachee.id), Some(SpeakerRole::Coachee)),
                ("Jim H (2)", None, None),
                ("Guest 1", None, None),
                ("cbourg2", Some(f.coachee.id), Some(SpeakerRole::Coachee)),
            ]
        );

        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn the_coach_export_keeps_only_attributed_coach_lines() -> Result<(), Error> {
    within_time_limit(async {
        let f = fixture().await;

        let rendered =
            export_plain_text(&f.db, &f.session, f.transcription_id, &[SpeakerRole::Coach]).await?;

        assert!(
            rendered.body.contains("[0:00] Jim H: "),
            "{}",
            rendered.body
        );
        assert!(!rendered.body.contains("[0:09]"), "{}", rendered.body);

        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn another_sessions_transcription_reads_as_empty() -> Result<(), Error> {
    within_time_limit(async {
        let f = fixture().await;

        let segments = read_segments(&f.db, &f.session, f.other_transcription_id).await?;

        assert!(segments.is_empty());

        Ok::<(), Error>(())
    })
    .await
}
