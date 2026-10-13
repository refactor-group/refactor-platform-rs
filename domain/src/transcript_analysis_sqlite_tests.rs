//! Analysis claiming against a real SQL engine. The claim is the multi-replica guard, so
//! what the UPDATE leaves in the table is the contract.

use chrono::{DateTime, Duration, FixedOffset, Utc};
use entity::transcript_analysis;
use entity::transcript_suggestion_target::Target;
use entity::Id;
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Set};

use super::*;
use crate::error::Error;
use crate::test_utils::sqlite::{database, seed_transcription, within_time_limit};

const MAX_ATTEMPTS: i16 = 3;

fn at(offset_minutes: i64) -> DateTime<FixedOffset> {
    (Utc::now() + Duration::minutes(offset_minutes)).into()
}

/// Inserts an analysis with the given state and returns its id.
async fn seed_analysis(
    db: &DatabaseConnection,
    transcription_id: Id,
    status: Status,
    attempts: i16,
    claimed_at: Option<DateTime<FixedOffset>>,
    created_at: DateTime<FixedOffset>,
) -> Id {
    let id = Id::new_v4();
    transcript_analysis::ActiveModel {
        id: Set(id),
        transcription_id: Set(transcription_id),
        target: Set(Target::Action),
        status: Set(status),
        attempts: Set(attempts),
        claimed_at: Set(claimed_at),
        error_message: Set(None),
        model: Set(None),
        input_tokens: Set(None),
        output_tokens: Set(None),
        created_at: Set(created_at),
        updated_at: Set(created_at),
    }
    .insert(db)
    .await
    .expect("the analysis is seeded");
    id
}

async fn reread(db: &DatabaseConnection, id: Id) -> Model {
    transcript_analysis::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the analysis is read")
        .expect("the analysis exists")
}

#[tokio::test]
async fn find_claimable_returns_only_pending_and_stale_processing_rows_with_attempts_left() {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db, 1).await.transcription_id;
        let stale_before = at(-10);

        let stale = seed_analysis(
            &db,
            transcription_id,
            Status::Processing,
            1,
            Some(at(-20)),
            at(-50),
        )
        .await;
        let pending = seed_analysis(&db, transcription_id, Status::Pending, 0, None, at(-60)).await;
        seed_analysis(
            &db,
            transcription_id,
            Status::Processing,
            1,
            Some(at(-5)),
            at(-40),
        )
        .await;
        seed_analysis(
            &db,
            transcription_id,
            Status::Completed,
            1,
            Some(at(-30)),
            at(-30),
        )
        .await;
        seed_analysis(&db, transcription_id, Status::Failed, 3, None, at(-20)).await;
        seed_analysis(
            &db,
            transcription_id,
            Status::Pending,
            MAX_ATTEMPTS,
            None,
            at(-10),
        )
        .await;

        let found = find_claimable(&db, stale_before, MAX_ATTEMPTS, 10).await?;
        let ids: Vec<Id> = found.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![pending, stale], "oldest created_at first");

        let limited = find_claimable(&db, stale_before, MAX_ATTEMPTS, 1).await?;
        assert_eq!(
            limited.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![pending]
        );

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn claim_moves_a_pending_row_to_processing_and_counts_the_attempt() {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db, 1).await.transcription_id;
        let id = seed_analysis(&db, transcription_id, Status::Pending, 0, None, at(-5)).await;
        let now = at(0);

        let claimed = claim(&db, id, now, at(-10), MAX_ATTEMPTS).await?;
        assert!(claimed.is_some());

        let row = reread(&db, id).await;
        assert_eq!(row.status, Status::Processing);
        assert_eq!(row.attempts, 1);
        assert_eq!(row.claimed_at, Some(now));

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn a_second_claim_of_the_same_row_gets_nothing_and_leaves_attempts_alone() {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db, 1).await.transcription_id;
        let id = seed_analysis(&db, transcription_id, Status::Pending, 0, None, at(-5)).await;
        let (now, stale_before) = (at(0), at(-10));

        assert!(claim(&db, id, now, stale_before, MAX_ATTEMPTS)
            .await?
            .is_some());
        assert!(claim(&db, id, now, stale_before, MAX_ATTEMPTS)
            .await?
            .is_none());

        assert_eq!(reread(&db, id).await.attempts, 1);

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn claim_retakes_a_stale_row_but_not_one_out_of_attempts() {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db, 1).await.transcription_id;
        let stale = seed_analysis(
            &db,
            transcription_id,
            Status::Processing,
            1,
            Some(at(-20)),
            at(-30),
        )
        .await;
        let spent = seed_analysis(
            &db,
            transcription_id,
            Status::Pending,
            MAX_ATTEMPTS,
            None,
            at(-30),
        )
        .await;
        let now = at(0);

        assert!(claim(&db, stale, now, at(-10), MAX_ATTEMPTS)
            .await?
            .is_some());
        let row = reread(&db, stale).await;
        assert_eq!(row.attempts, 2);
        assert_eq!(row.claimed_at, Some(now));

        assert!(claim(&db, spent, now, at(-10), MAX_ATTEMPTS)
            .await?
            .is_none());
        let row = reread(&db, spent).await;
        assert_eq!(row.status, Status::Pending);
        assert_eq!(row.attempts, MAX_ATTEMPTS);

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

/// A processing analysis claimed with `attempts`, for its own fresh transcription.
async fn seed_claimed(db: &DatabaseConnection, attempts: i16) -> Id {
    let transcription_id = seed_transcription(db, 1).await.transcription_id;
    seed_analysis(
        db,
        transcription_id,
        Status::Processing,
        attempts,
        Some(at(-1)),
        at(-5),
    )
    .await
}

#[tokio::test]
async fn release_fail_and_complete_leave_the_expected_state_in_the_table() {
    within_time_limit(async {
        let db = database().await;

        let released = seed_claimed(&db, 2).await;
        assert!(
            release_for_retry(&db, released, 2, "timeout".to_string(), at(0))
                .await?
                .is_some()
        );
        let row = reread(&db, released).await;
        assert_eq!(row.status, Status::Pending);
        assert_eq!(row.claimed_at, None);
        assert_eq!(row.error_message.as_deref(), Some("timeout"));
        assert_eq!(row.attempts, 2);

        let failed = seed_claimed(&db, 3).await;
        assert!(fail(&db, failed, 3, "gave up".to_string(), at(0))
            .await?
            .is_some());
        let row = reread(&db, failed).await;
        assert_eq!(row.status, Status::Failed);
        assert_eq!(row.error_message.as_deref(), Some("gave up"));
        assert_eq!(row.attempts, 3);

        complete(&db, failed, "model-x".to_string(), Some(120), Some(30)).await?;
        let row = reread(&db, failed).await;
        assert_eq!(row.status, Status::Completed);
        assert_eq!(row.model.as_deref(), Some("model-x"));
        assert_eq!(row.input_tokens, Some(120));
        assert_eq!(row.output_tokens, Some(30));
        assert_eq!(row.error_message, None);

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn a_worker_whose_claim_was_retaken_cannot_release_or_fail_the_analysis() {
    within_time_limit(async {
        let db = database().await;

        // Another worker retook the claim, so the attempts no longer match.
        let retaken = seed_claimed(&db, 2).await;
        assert!(
            release_for_retry(&db, retaken, 1, "timeout".to_string(), at(0))
                .await?
                .is_none()
        );
        assert!(fail(&db, retaken, 1, "gave up".to_string(), at(0))
            .await?
            .is_none());
        let row = reread(&db, retaken).await;
        assert_eq!((row.status, row.attempts), (Status::Processing, 2));
        assert_eq!(row.error_message, None);

        // Another worker finished it, with the same attempts the caller counted.
        let finished = seed_claimed(&db, 2).await;
        complete_claimed(&db, finished, 2, "model-x".to_string(), None, None, at(0)).await?;
        assert!(
            release_for_retry(&db, finished, 2, "timeout".to_string(), at(0))
                .await?
                .is_none()
        );
        assert!(fail(&db, finished, 2, "gave up".to_string(), at(0))
            .await?
            .is_none());
        let row = reread(&db, finished).await;
        assert_eq!(row.status, Status::Completed);
        assert_eq!(row.error_message, None);

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn fail_abandoned_fails_only_stale_claims_with_no_attempts_left() {
    within_time_limit(async {
        let db = database().await;
        let seed = |status, attempts, claimed_at| {
            let db = &db;
            async move {
                let transcription_id = seed_transcription(db, 1).await.transcription_id;
                seed_analysis(db, transcription_id, status, attempts, claimed_at, at(-60)).await
            }
        };
        let abandoned = seed(Status::Processing, MAX_ATTEMPTS, Some(at(-30))).await;
        let still_running = seed(Status::Processing, MAX_ATTEMPTS, Some(at(-1))).await;
        let retakeable = seed(Status::Processing, MAX_ATTEMPTS - 1, Some(at(-30))).await;
        let pending = seed(Status::Pending, MAX_ATTEMPTS, None).await;
        let completed = seed(Status::Completed, MAX_ATTEMPTS, Some(at(-30))).await;

        let failed =
            fail_abandoned(&db, at(-10), MAX_ATTEMPTS, "abandoned".to_string(), at(0)).await?;

        assert_eq!(
            failed.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![abandoned]
        );
        let row = reread(&db, abandoned).await;
        assert_eq!(row.status, Status::Failed);
        assert_eq!(row.error_message.as_deref(), Some("abandoned"));
        for (id, status) in [
            (still_running, Status::Processing),
            (retakeable, Status::Processing),
            (pending, Status::Pending),
            (completed, Status::Completed),
        ] {
            assert_eq!(reread(&db, id).await.status, status);
        }

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn complete_claimed_completes_only_the_claim_with_matching_attempts() {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db, 1).await.transcription_id;
        let current = seed_analysis(
            &db,
            transcription_id,
            Status::Processing,
            2,
            Some(at(-1)),
            at(-5),
        )
        .await;
        let retaken = seed_analysis(
            &db,
            transcription_id,
            Status::Processing,
            2,
            Some(at(-1)),
            at(-5),
        )
        .await;
        let now = at(0);

        let completed = complete_claimed(
            &db,
            current,
            2,
            "model-x".to_string(),
            Some(120),
            Some(30),
            now,
        )
        .await?;
        assert!(completed.is_some());
        let row = reread(&db, current).await;
        assert_eq!(row.status, Status::Completed);
        assert_eq!(row.model.as_deref(), Some("model-x"));
        assert_eq!(row.input_tokens, Some(120));
        assert_eq!(row.output_tokens, Some(30));
        assert_eq!(row.updated_at, now);

        let before = reread(&db, retaken).await;
        let superseded = complete_claimed(
            &db,
            retaken,
            1,
            "model-x".to_string(),
            Some(120),
            Some(30),
            now,
        )
        .await?;
        assert!(superseded.is_none());
        assert_eq!(reread(&db, retaken).await, before, "the row is unchanged");

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn a_second_analysis_for_the_same_transcription_and_target_is_rejected() {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db, 1).await.transcription_id;
        // Schema sync builds tables from entities, so mirror the migration's unique constraint.
        db.execute_unprepared(
            "CREATE UNIQUE INDEX refactor_platform.transcript_analyses_transcription_target_unique \
             ON transcript_analyses (transcription_id, target)",
        )
        .await
        .expect("the unique index is created");

        create_pending(&db, transcription_id, Target::Action).await?;
        let duplicate = create_pending(&db, transcription_id, Target::Action).await;
        assert!(duplicate.is_err());

        let rows = transcript_analysis::Entity::find()
            .all(&db)
            .await
            .expect("the table is read");
        assert_eq!(rows.len(), 1);

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}
