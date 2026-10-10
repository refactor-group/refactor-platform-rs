//! The sweep against a real SQL engine with a fake model: what each failure leaves in the
//! analyses table, and that one failure does not stop the rest.

use std::sync::Arc;

use async_trait::async_trait;
use entity::{coaching_sessions, transcript_analysis};
use meeting_ai::types::analysis::{Completion, Request, Usage};
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Set};
use serde_json::json;

use super::*;
use crate::test_utils::sqlite::{database, seed_transcription, within_time_limit};
use crate::transcript_analysis::{create_pending, Status};
use crate::Id;

/// Answers with an empty suggestion list, or fails with the error `fail` builds.
struct FakeModel {
    fail: Option<fn() -> meeting_ai::Error>,
}

#[async_trait]
impl analysis::Provider for FakeModel {
    async fn complete(&self, _request: Request) -> Result<Completion, meeting_ai::Error> {
        match self.fail {
            Some(fail) => Err(fail()),
            None => Ok(Completion {
                output: json!({ "suggestions": [] }),
                model: "fake-model".to_string(),
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 2,
                },
                request_id: None,
            }),
        }
    }

    fn provider_id(&self) -> &str {
        "fake"
    }
}

fn sweep(fail: Option<fn() -> meeting_ai::Error>) -> Sweep {
    Sweep {
        provider: Arc::new(FakeModel { fail }),
        publisher: EventPublisher::new(),
    }
}

fn context(db: DatabaseConnection) -> Context {
    Context {
        db: Arc::new(db),
        config: Config::from_args(["test"]),
    }
}

/// A pending analysis for a fresh transcription with one segment, at `attempts` so far.
async fn seed_pending(db: &DatabaseConnection, attempts: i16) -> (Id, Id) {
    let seeded = seed_transcription(db, 1).await;
    let pending = create_pending(db, seeded.transcription_id, Target::Action)
        .await
        .expect("the analysis is created");
    transcript_analysis::ActiveModel {
        id: Set(pending.id),
        attempts: Set(attempts),
        ..Default::default()
    }
    .update(db)
    .await
    .expect("the attempts are set");
    (pending.id, seeded.session_id)
}

async fn reread(ctx: &Context, id: Id) -> transcript_analysis::Model {
    transcript_analysis::Entity::find_by_id(id)
        .one(ctx.db.as_ref())
        .await
        .expect("the analysis is read")
        .expect("the analysis exists")
}

fn authentication() -> meeting_ai::Error {
    meeting_ai::Error::Authentication("bad key".to_string())
}

fn network() -> meeting_ai::Error {
    meeting_ai::Error::Network("connection reset".to_string())
}

#[tokio::test]
async fn an_authentication_error_fails_the_analysis_for_good() {
    within_time_limit(async {
        let db = database().await;
        let (id, _) = seed_pending(&db, 0).await;
        let ctx = context(db);

        sweep(Some(authentication))
            .run(&ctx)
            .await
            .expect("the tick runs");

        let row = reread(&ctx, id).await;
        assert_eq!(row.status, Status::Failed);
        assert_eq!(row.attempts, 1);
    })
    .await;
}

#[tokio::test]
async fn a_network_error_on_the_first_attempt_is_released_for_retry() {
    within_time_limit(async {
        let db = database().await;
        let (id, _) = seed_pending(&db, 0).await;
        let ctx = context(db);

        sweep(Some(network)).run(&ctx).await.expect("the tick runs");

        let row = reread(&ctx, id).await;
        assert_eq!(row.status, Status::Pending);
        assert_eq!(row.claimed_at, None);
        assert_eq!(row.attempts, 1);
        let message = row.error_message.expect("the error is stored");
        assert!(message.contains("Network"));
        assert!(
            !message.contains("connection reset"),
            "only the variant is stored"
        );
    })
    .await;
}

#[tokio::test]
async fn a_network_error_on_the_last_attempt_fails_the_analysis() {
    within_time_limit(async {
        let db = database().await;
        let (id, _) = seed_pending(&db, 2).await;
        let ctx = context(db);

        sweep(Some(network)).run(&ctx).await.expect("the tick runs");

        let row = reread(&ctx, id).await;
        assert_eq!(row.attempts, 3);
        assert_eq!(row.status, Status::Failed);
    })
    .await;
}

#[tokio::test]
async fn a_failing_analysis_does_not_stop_the_next_one() {
    within_time_limit(async {
        let db = database().await;
        let (broken, broken_session) = seed_pending(&db, 0).await;
        let (healthy, _) = seed_pending(&db, 0).await;
        // Orphan the first analysis's transcription so loading its session fails.
        db.execute_unprepared("PRAGMA foreign_keys = OFF")
            .await
            .expect("foreign keys are paused");
        coaching_sessions::Entity::delete_by_id(broken_session)
            .exec(&db)
            .await
            .expect("the session is deleted");
        db.execute_unprepared("PRAGMA foreign_keys = ON")
            .await
            .expect("foreign keys are restored");
        let ctx = context(db);

        let outcome = sweep(None).run(&ctx).await.expect("the tick runs");

        assert_eq!((outcome.processed, outcome.attempted), (1, 2));
        assert_eq!(reread(&ctx, broken).await.status, Status::Failed);
        assert_eq!(reread(&ctx, healthy).await.status, Status::Completed);
    })
    .await;
}
