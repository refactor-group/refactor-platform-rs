//! Transcript participant storage against a real SQL engine. Attribution lives on these
//! rows, so a lost column or a wrong cascade is a mislabeled transcript.

use chrono::Utc;
use entity::meeting_recording::{self, MeetingRecordingStatus};
use entity::transcription::{self, TranscriptionStatus};
use entity::{transcript_participant, transcript_segment, users, Id};
use entity_api::transcript_segment as segment_api;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use serde_json::json;

use super::*;
use crate::error::Error;
use crate::test_utils::sqlite::{database, seed_coaching_session, seed_user, within_time_limit};

/// Records a completed transcription in a fresh session and returns its id.
async fn seed_transcription(db: &DatabaseConnection) -> Id {
    let (coaching_session_id, _) = seed_coaching_session(db).await;
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
    transcription::ActiveModel {
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

/// An attributed participant with every optional column populated.
fn attributed(transcription_id: Id, user_id: Id) -> transcript_participant::ActiveModel {
    transcript_participant::ActiveModel {
        id: Set(Id::new_v4()),
        transcription_id: Set(transcription_id),
        provider_participant_id: Set("100".to_string()),
        display_name: Set(Some("J. Hodapp".to_string())),
        is_host: Set(Some(true)),
        platform: Set(Some("desktop".to_string())),
        platform_account_id: Set(Some("conf-user-1".to_string())),
        extra_data: Set(Some(json!({ "zoom": { "conf_user_id": "conf-user-1" } }))),
        user_id: Set(Some(user_id)),
        match_source: Set(Some(MatchSource::Account)),
        created_at: Set(Utc::now().into()),
    }
}

/// An unattributed participant with every optional column empty.
fn unattributed(transcription_id: Id, provider_id: &str) -> transcript_participant::ActiveModel {
    transcript_participant::ActiveModel {
        id: Set(Id::new_v4()),
        transcription_id: Set(transcription_id),
        provider_participant_id: Set(provider_id.to_string()),
        display_name: Set(None),
        is_host: Set(None),
        platform: Set(None),
        platform_account_id: Set(None),
        extra_data: Set(None),
        user_id: Set(None),
        match_source: Set(None),
        created_at: Set(Utc::now().into()),
    }
}

/// Every column survives the write, and a read returns only the named transcription's rows.
#[tokio::test]
async fn participants_round_trip_and_stay_with_their_transcription() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let (mine, other) = (seed_transcription(&db).await, seed_transcription(&db).await);
        let user_id = seed_user(&db).await;

        let written = create_batch(
            &db,
            vec![attributed(mine, user_id), unattributed(mine, "200")],
        )
        .await?;
        create_batch(&db, vec![unattributed(other, "300")]).await?;

        let mut read = find_by_transcription(&db, mine).await?;
        read.sort_by(|a, b| a.provider_participant_id.cmp(&b.provider_participant_id));

        assert_eq!(read.len(), 2);
        assert_eq!(written.len(), 2);

        let coach = &read[0];
        assert_eq!(coach.transcription_id, mine);
        assert_eq!(coach.provider_participant_id, "100");
        assert_eq!(coach.display_name.as_deref(), Some("J. Hodapp"));
        assert_eq!(coach.is_host, Some(true));
        assert_eq!(coach.platform.as_deref(), Some("desktop"));
        assert_eq!(coach.platform_account_id.as_deref(), Some("conf-user-1"));
        assert_eq!(
            coach.extra_data,
            Some(json!({ "zoom": { "conf_user_id": "conf-user-1" } }))
        );
        assert_eq!(coach.user_id, Some(user_id));
        assert_eq!(coach.match_source, Some(MatchSource::Account));

        let guest = &read[1];
        assert_eq!(guest.provider_participant_id, "200");
        assert_eq!(guest.display_name, None);
        assert_eq!(guest.is_host, None);
        assert_eq!(guest.platform, None);
        assert_eq!(guest.platform_account_id, None);
        assert_eq!(guest.extra_data, None);
        assert_eq!(guest.user_id, None);
        assert_eq!(guest.match_source, None);

        Ok::<(), Error>(())
    })
    .await
}

/// Both match sources are stored as distinct values.
#[tokio::test]
async fn match_sources_are_stored_distinctly() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db).await;
        let (coach_id, coachee_id) = (seed_user(&db).await, seed_user(&db).await);

        let mut coachee = attributed(transcription_id, coachee_id);
        coachee.provider_participant_id = Set("200".to_string());
        coachee.match_source = Set(Some(MatchSource::Elimination));
        create_batch(&db, vec![attributed(transcription_id, coach_id), coachee]).await?;

        let read = find_by_transcription(&db, transcription_id).await?;
        let source_of = |user_id: Id| {
            read.iter()
                .find(|participant| participant.user_id == Some(user_id))
                .and_then(|participant| participant.match_source)
        };

        assert_eq!(source_of(coach_id), Some(MatchSource::Account));
        assert_eq!(source_of(coachee_id), Some(MatchSource::Elimination));

        Ok::<(), Error>(())
    })
    .await
}

/// An empty batch is a no-op rather than an invalid empty INSERT.
#[tokio::test]
async fn an_empty_batch_writes_nothing() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db).await;

        assert!(create_batch(&db, vec![]).await?.is_empty());
        assert!(find_by_transcription(&db, transcription_id)
            .await?
            .is_empty());

        Ok::<(), Error>(())
    })
    .await
}

/// A segment keeps the participant that spoke it.
#[tokio::test]
async fn a_segment_keeps_its_participant() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db).await;
        let participant = create_batch(&db, vec![unattributed(transcription_id, "200")])
            .await?
            .remove(0);

        segment_api::create_batch(
            &db,
            vec![transcript_segment::ActiveModel {
                id: Set(Id::new_v4()),
                transcription_id: Set(transcription_id),
                participant_id: Set(Some(participant.id)),
                speaker_label: Set("Caleb".to_string()),
                text: Set("Morning.".to_string()),
                start_ms: Set(0),
                end_ms: Set(1000),
                confidence: Set(None),
                sentiment: Set(None),
                created_at: Set(Utc::now().into()),
            }],
        )
        .await?;

        let segments = segment_api::find_by_transcription(&db, transcription_id).await?;
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].participant_id, Some(participant.id));

        Ok::<(), Error>(())
    })
    .await
}

/// Deleting a transcription removes its participants and leaves other transcriptions alone.
#[tokio::test]
async fn deleting_a_transcription_removes_its_participants() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let (doomed, kept) = (seed_transcription(&db).await, seed_transcription(&db).await);
        create_batch(&db, vec![unattributed(doomed, "200")]).await?;
        create_batch(&db, vec![unattributed(kept, "200")]).await?;

        transcription::Entity::delete_by_id(doomed)
            .exec(&db)
            .await
            .expect("the transcription is deleted");

        assert!(find_by_transcription(&db, doomed).await?.is_empty());
        assert_eq!(find_by_transcription(&db, kept).await?.len(), 1);

        Ok::<(), Error>(())
    })
    .await
}

/// Deleting an attributed user keeps the participant but clears its user.
#[tokio::test]
async fn deleting_an_attributed_user_clears_only_the_user() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db).await;
        let user_id = seed_user(&db).await;
        create_batch(&db, vec![attributed(transcription_id, user_id)]).await?;

        users::Entity::delete_by_id(user_id)
            .exec(&db)
            .await
            .expect("the user is deleted");

        let read = find_by_transcription(&db, transcription_id).await?;
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].user_id, None);
        assert_eq!(read[0].match_source, Some(MatchSource::Account));

        Ok::<(), Error>(())
    })
    .await
}
