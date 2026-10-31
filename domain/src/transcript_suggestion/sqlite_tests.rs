//! Suggestion storage against a real SQL engine. Review state lives in these rows, so what
//! each write leaves in the table is the contract.

use chrono::{DateTime, Duration, FixedOffset, Utc};
use entity::transcript_suggestion;
use entity::transcript_suggestion_target::Target;
use entity::{coaching_sessions, Id};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, PaginatorTrait, Set};
use serde_json::json;

use super::*;
use crate::error::Error;
use crate::test_utils::sqlite::{database, seed_transcription, within_time_limit};
use crate::transcript_analysis;

fn at(offset_minutes: i64) -> DateTime<FixedOffset> {
    (Utc::now() + Duration::minutes(offset_minutes)).into()
}

/// A pending suggestion row for the given parents.
fn pending_row(
    session_id: Id,
    analysis_id: Id,
    segment_id: Id,
    proposal: serde_json::Value,
    created_at: DateTime<FixedOffset>,
) -> ActiveModel {
    ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        transcript_analysis_id: Set(analysis_id),
        target: Set(Target::Action),
        proposal: Set(proposal),
        segment_id: Set(segment_id),
        quote: Set("I will send the notes".to_string()),
        duplicate_of_id: Set(None),
        status: Set(Status::Pending),
        resolved_by: Set(None),
        resolved_at: Set(None),
        created_entity_id: Set(None),
        created_at: Set(created_at),
        updated_at: Set(created_at),
    }
}

/// Seeds a transcription with one segment and an analysis, and returns a row factory's parents.
async fn seed_parents(db: &DatabaseConnection) -> (Id, Id, Id, Id) {
    let seeded = seed_transcription(db, 1).await;
    let analysis = transcript_analysis::create_pending(db, seeded.transcription_id, Target::Action)
        .await
        .expect("the analysis is seeded");
    (
        seeded.session_id,
        analysis.id,
        seeded.segment_ids[0],
        seeded.user_id,
    )
}

async fn reread(db: &DatabaseConnection, id: Id) -> Model {
    transcript_suggestion::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the suggestion is read")
        .expect("the suggestion exists")
}

async fn insert_one(db: &DatabaseConnection, row: ActiveModel) -> Model {
    row.insert(db).await.expect("the suggestion is seeded")
}

#[tokio::test]
async fn create_batch_inserts_every_row_and_keeps_the_proposal_json() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, analysis_id, segment_id, _) = seed_parents(&db).await;

        assert!(create_batch(&db, vec![]).await?.is_empty());

        let proposals = [
            json!({ "body": "Send notes", "assignees": ["a", "b"], "due_by": null }),
            json!({ "body": "Book room", "nested": { "n": 1.5, "ok": true } }),
            json!({ "body": "Call back" }),
        ];
        let rows = proposals
            .iter()
            .enumerate()
            .map(|(i, p)| pending_row(session_id, analysis_id, segment_id, p.clone(), at(i as i64)))
            .collect();

        let inserted = create_batch(&db, rows).await?;
        assert_eq!(inserted.len(), 3);

        let count = transcript_suggestion::Entity::find()
            .count(&db)
            .await
            .expect("the rows are counted");
        assert_eq!(count, 3);
        for (model, proposal) in inserted.iter().zip(&proposals) {
            assert_eq!(&reread(&db, model.id).await.proposal, proposal);
        }

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn find_by_session_skips_other_sessions_and_resolved_rows_in_created_order() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, analysis_id, segment_id, user_id) = seed_parents(&db).await;
        let (other_session, other_analysis, other_segment, _) = seed_parents(&db).await;

        let later = insert_one(
            &db,
            pending_row(session_id, analysis_id, segment_id, json!({"n": 2}), at(-1)),
        )
        .await;
        let earlier = insert_one(
            &db,
            pending_row(session_id, analysis_id, segment_id, json!({"n": 1}), at(-5)),
        )
        .await;
        insert_one(
            &db,
            pending_row(
                other_session,
                other_analysis,
                other_segment,
                json!({"n": 3}),
                at(-3),
            ),
        )
        .await;
        let accepted = insert_one(
            &db,
            pending_row(session_id, analysis_id, segment_id, json!({"n": 4}), at(-4)),
        )
        .await;
        let dismissed = insert_one(
            &db,
            pending_row(session_id, analysis_id, segment_id, json!({"n": 5}), at(-2)),
        )
        .await;
        transition(
            &db,
            accepted.id,
            Status::Pending,
            Status::Accepted,
            user_id,
            None,
            at(0),
        )
        .await?;
        transition(
            &db,
            dismissed.id,
            Status::Pending,
            Status::Dismissed,
            user_id,
            None,
            at(0),
        )
        .await?;

        let found = find_by_session(&db, session_id, Target::Action, Status::Pending, 10).await?;
        assert_eq!(
            found.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![earlier.id, later.id]
        );

        let first = find_by_session(&db, session_id, Target::Action, Status::Pending, 1).await?;
        assert_eq!(
            first.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![earlier.id]
        );

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn find_by_session_lists_a_row_that_created_an_entity_only_as_accepted() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, analysis_id, segment_id, user_id) = seed_parents(&db).await;
        let row = insert_one(
            &db,
            pending_row(session_id, analysis_id, segment_id, json!({"n": 1}), at(-5)),
        )
        .await;
        transition(
            &db,
            row.id,
            Status::Pending,
            Status::Accepted,
            user_id,
            Some(Id::new_v4()),
            at(0),
        )
        .await?;
        let ids = |status| {
            let db = &db;
            async move {
                find_by_session(db, session_id, Target::Action, status, 10)
                    .await
                    .map(|rows| rows.into_iter().map(|m| m.id).collect::<Vec<_>>())
            }
        };

        for status in [Status::Pending, Status::Dismissed] {
            ActiveModel {
                id: Set(row.id),
                status: Set(status),
                ..Default::default()
            }
            .update(&db)
            .await
            .expect("the status is set");
            assert!(ids(status).await?.is_empty(), "{status:?}");
        }

        ActiveModel {
            id: Set(row.id),
            status: Set(Status::Accepted),
            ..Default::default()
        }
        .update(&db)
        .await
        .expect("the status is set");
        assert_eq!(ids(Status::Accepted).await?, vec![row.id]);

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn accepting_records_the_resolution_and_a_second_accept_gets_nothing() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, analysis_id, segment_id, user_id) = seed_parents(&db).await;
        let row = insert_one(
            &db,
            pending_row(session_id, analysis_id, segment_id, json!({}), at(-5)),
        )
        .await;
        let created = Id::new_v4();
        let now = at(0);

        let first = transition(
            &db,
            row.id,
            Status::Pending,
            Status::Accepted,
            user_id,
            Some(created),
            now,
        )
        .await?;
        assert!(first.is_some());

        let stored = reread(&db, row.id).await;
        assert_eq!(stored.status, Status::Accepted);
        assert_eq!(stored.resolved_by, Some(user_id));
        assert_eq!(stored.resolved_at, Some(now));
        assert_eq!(stored.created_entity_id, Some(created));

        let second = transition(
            &db,
            row.id,
            Status::Pending,
            Status::Accepted,
            user_id,
            Some(Id::new_v4()),
            at(1),
        )
        .await?;
        assert!(second.is_none());
        assert_eq!(reread(&db, row.id).await.created_entity_id, Some(created));

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn restoring_clears_the_resolution_and_a_mismatched_from_changes_nothing() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, analysis_id, segment_id, user_id) = seed_parents(&db).await;
        let row = insert_one(
            &db,
            pending_row(session_id, analysis_id, segment_id, json!({}), at(-5)),
        )
        .await;
        transition(
            &db,
            row.id,
            Status::Pending,
            Status::Dismissed,
            user_id,
            Some(Id::new_v4()),
            at(0),
        )
        .await?;

        let mismatch = transition(
            &db,
            row.id,
            Status::Accepted,
            Status::Pending,
            user_id,
            None,
            at(1),
        )
        .await?;
        assert!(mismatch.is_none());
        assert_eq!(reread(&db, row.id).await.status, Status::Dismissed);

        let restored = transition(
            &db,
            row.id,
            Status::Dismissed,
            Status::Pending,
            user_id,
            None,
            at(2),
        )
        .await?;
        assert!(restored.is_some());
        let stored = reread(&db, row.id).await;
        assert_eq!(stored.status, Status::Pending);
        assert_eq!(stored.resolved_by, None);
        assert_eq!(stored.resolved_at, None);
        assert_eq!(stored.created_entity_id, None);

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}

#[tokio::test]
async fn deleting_the_coaching_session_removes_its_suggestions() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, analysis_id, segment_id, _) = seed_parents(&db).await;
        insert_one(
            &db,
            pending_row(session_id, analysis_id, segment_id, json!({}), at(-5)),
        )
        .await;

        coaching_sessions::Entity::delete_by_id(session_id)
            .exec(&db)
            .await
            .expect("the session is deleted");

        let count = transcript_suggestion::Entity::find()
            .count(&db)
            .await
            .expect("the rows are counted");
        assert_eq!(count, 0);

        Ok::<(), Error>(())
    })
    .await
    .expect("the test body succeeds");
}
