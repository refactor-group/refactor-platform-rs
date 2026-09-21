use super::*;
use crate::coaching_relationships;
use chrono::{NaiveDate, Utc};
use entity::transcript_segment::Model as Segment;
use sea_orm::{DatabaseBackend, MockDatabase};

fn session(coaching_relationship_id: Id) -> coaching_sessions::Model {
    let now = Utc::now();
    coaching_sessions::Model {
        id: Id::new_v4(),
        coaching_relationship_id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: NaiveDate::from_ymd_opt(2026, 9, 21)
            .and_then(|date| date.and_hms_opt(10, 0, 0))
            .unwrap_or_default(),
        duration_minutes: 60,
        title: None,
        meeting_url: None,
        provider: None,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: None,
        notice_given_at: now.into(),
    }
}

fn relationship(coach_id: Id, coachee_id: Id) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        id: Id::new_v4(),
        organization_id: Id::new_v4(),
        coach_id,
        coachee_id,
        slug: "jim-caleb".to_owned(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn user(first: &str, last: &str, display: Option<&str>) -> users::Model {
    let now = Utc::now();
    users::Model {
        id: Id::new_v4(),
        email: format!("{}@test.com", first.to_lowercase()),
        first_name: first.to_owned(),
        last_name: last.to_owned(),
        display_name: display.map(str::to_owned),
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

fn coach() -> users::Model {
    user("Jim", "Hodapp", Some("Jim H"))
}

fn coachee() -> users::Model {
    user("Caleb", "Bourg", None)
}

fn transcription(coaching_session_id: Id, status: TranscriptionStatus) -> Model {
    let now = Utc::now();
    Model {
        id: Id::new_v4(),
        coaching_session_id,
        meeting_recording_id: Id::new_v4(),
        external_id: "external-1".to_owned(),
        recall_recording_id: Some("recording-1".to_owned()),
        status,
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

fn segment(transcription_id: Id, label: &str, text: &str, start_ms: i32) -> Segment {
    Segment {
        id: Id::new_v4(),
        transcription_id,
        speaker_label: label.to_owned(),
        text: text.to_owned(),
        start_ms,
        end_ms: start_ms + 1000,
        confidence: None,
        sentiment: None,
        created_at: Utc::now().into(),
    }
}

fn segments(transcription_id: Id) -> Vec<Segment> {
    vec![
        segment(transcription_id, "Jim H", "Good morning.", 0),
        segment(transcription_id, "Caleb Bourg", "Morning.", 4000),
        segment(transcription_id, "Guest", "Hi both.", 9000),
    ]
}

fn not_found() -> DomainErrorKind {
    DomainErrorKind::Internal(InternalErrorKind::Entity(
        EntityErrorKind::TranscriptionNotFound,
    ))
}

/// Every statement the mock saw, in order.
fn statements(db: DatabaseConnection) -> Vec<String> {
    db.into_transaction_log()
        .iter()
        .flat_map(|transaction| {
            transaction
                .statements()
                .iter()
                .map(|statement| statement.sql.clone())
        })
        .collect()
}

#[tokio::test]
async fn find_for_session_rejects_a_missing_transcription() {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<Model>::new()])
        .into_connection();

    let error = find_for_session(&db, Id::new_v4(), Id::new_v4())
        .await
        .expect_err("a missing transcription must not resolve");

    assert_eq!(error.error_kind, not_found());
}

#[tokio::test]
async fn find_for_session_rejects_a_transcription_under_another_session() {
    let row = transcription(Id::new_v4(), TranscriptionStatus::Completed);
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[row.clone()]])
        .into_connection();

    let error = find_for_session(&db, row.id, Id::new_v4())
        .await
        .expect_err("another session's transcription must not resolve");

    assert_eq!(error.error_kind, not_found());
}

#[tokio::test]
async fn export_plain_text_refuses_every_non_completed_status_before_reading_segments() {
    let session = session(Id::new_v4());

    for status in [
        TranscriptionStatus::Queued,
        TranscriptionStatus::Processing,
        TranscriptionStatus::Failed,
    ] {
        let row = transcription(session.id, status.clone());
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([[row.clone()]])
            .into_connection();

        let error = export_plain_text(&db, &session, row.id, &[])
            .await
            .expect_err("an incomplete transcription must not export");

        assert_eq!(
            error.error_kind,
            DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::TranscriptionNotCompleted,
            ))
        );
        assert_eq!(statements(db).len(), 1, "status {status:?} queried further");
    }
}

#[tokio::test]
async fn export_plain_text_renders_the_relationship_participants() {
    let coach = coach();
    let coachee = coachee();
    let relationship = relationship(coach.id, coachee.id);
    let session = session(relationship.id);
    let row = transcription(session.id, TranscriptionStatus::Completed);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[row.clone()]])
        .append_query_results([segments(row.id)])
        .append_query_results([[relationship]])
        .append_query_results([[coach]])
        .append_query_results([[coachee]])
        .into_connection();

    let rendered = export_plain_text(&db, &session, row.id, &[SpeakerRole::Coach])
        .await
        .expect("the coach-only export should render");

    assert_eq!(rendered.filename, "transcript-2026-09-21-filtered.txt");
    assert!(rendered.body.contains("Speakers: Jim H\n"));
    assert!(rendered.body.contains("[0:00] Jim H: Good morning.\n"));
    assert!(!rendered.body.contains("Caleb Bourg"));
    assert!(!rendered.body.contains("Guest"));
}

#[tokio::test]
async fn export_plain_text_reports_an_unidentified_role() {
    let coach = coach();
    let coachee = user("Nobody", "Here", None);
    let relationship = relationship(coach.id, coachee.id);
    let session = session(relationship.id);
    let row = transcription(session.id, TranscriptionStatus::Completed);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[row.clone()]])
        .append_query_results([segments(row.id)])
        .append_query_results([[relationship]])
        .append_query_results([[coach]])
        .append_query_results([[coachee]])
        .into_connection();

    let error = export_plain_text(&db, &session, row.id, &[SpeakerRole::Coachee])
        .await
        .expect_err("a coachee with no matching label must not export");

    assert_eq!(
        error.error_kind,
        DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::SpeakerNotIdentified {
                role: SpeakerRole::Coachee,
                labels: vec![
                    "Jim H".to_owned(),
                    "Caleb Bourg".to_owned(),
                    "Guest".to_owned()
                ],
            }
        ))
    );
}

#[tokio::test]
async fn read_with_speakers_resolves_roles_in_appearance_order() {
    let coach = coach();
    let coachee = coachee();
    let relationship = relationship(coach.id, coachee.id);
    let session = session(relationship.id);
    let row = transcription(session.id, TranscriptionStatus::Queued);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[row.clone()]])
        .append_query_results([segments(row.id)])
        .append_query_results([[relationship]])
        .append_query_results([[coach]])
        .append_query_results([[coachee]])
        .into_connection();

    let result = read_with_speakers(&db, &session, row.id)
        .await
        .expect("a queued transcription should still read");

    let labels: Vec<&str> = result
        .speakers
        .iter()
        .map(|speaker| speaker.label.as_str())
        .collect();
    let roles: Vec<Option<SpeakerRole>> =
        result.speakers.iter().map(|speaker| speaker.role).collect();

    assert_eq!(labels, ["Jim H", "Caleb Bourg", "Guest"]);
    assert_eq!(
        roles,
        [Some(SpeakerRole::Coach), Some(SpeakerRole::Coachee), None]
    );
    assert_eq!(result.transcription.status, TranscriptionStatus::Queued);
}

#[tokio::test]
async fn read_with_speakers_is_empty_without_segments() {
    let coach = coach();
    let coachee = coachee();
    let relationship = relationship(coach.id, coachee.id);
    let session = session(relationship.id);
    let row = transcription(session.id, TranscriptionStatus::Completed);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[row.clone()]])
        .append_query_results([Vec::<Segment>::new()])
        .append_query_results([[relationship]])
        .append_query_results([[coach]])
        .append_query_results([[coachee]])
        .into_connection();

    let result = read_with_speakers(&db, &session, row.id)
        .await
        .expect("a transcription without segments should still read");

    assert!(result.speakers.is_empty());
}

#[tokio::test]
async fn with_speakers_serializes_flat() {
    let row = transcription(Id::new_v4(), TranscriptionStatus::Completed);
    let value = serde_json::to_value(WithSpeakers {
        transcription: row,
        speakers: vec![Speaker {
            label: "Jim H".to_owned(),
            role: Some(SpeakerRole::Coach),
        }],
    })
    .expect("WithSpeakers should serialize");

    assert!(value.get("id").is_some());
    assert!(value.get("status").is_some());
    assert!(value.get("coaching_session_id").is_some());
    assert!(value.get("speakers").is_some());
    assert!(value.get("transcription").is_none());
    assert!(value.get("recall_recording_id").is_none());
}
