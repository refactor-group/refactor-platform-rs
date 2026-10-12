//! The sweep against a real SQL engine with a fake model: what each failure leaves in the
//! analyses table, that one failure does not stop the rest, and what each outcome announces.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use entity::{coaching_sessions, transcript_analysis};
use meeting_ai::types::analysis::{Completion, Request, Usage};
use sea_orm::{ActiveModelTrait, ConnectionTrait, DatabaseConnection, EntityTrait, Set};
use serde_json::json;

use super::*;
use crate::events::{DomainEvent, EventHandler};
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

/// Answers with one suggestion for the coachee, sourced from the first segment.
struct OneSuggestion;

#[async_trait]
impl analysis::Provider for OneSuggestion {
    async fn complete(&self, _request: Request) -> Result<Completion, meeting_ai::Error> {
        Ok(Completion {
            output: json!({ "suggestions": [{
                "body": "Send the segment summary",
                "assignees": ["coachee"],
                "due_date": null,
                "due_phrase": null,
                "source_segment": "s1",
                "quote": "Segment 0",
                "duplicate_of": null,
            }]}),
            model: "fake-model".to_string(),
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
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

impl Recorder {
    /// Suggestion events for `session_id` with target `action`.
    fn action_changes(&self, session_id: Id) -> usize {
        self.0
            .lock()
            .expect("the lock is free")
            .iter()
            .filter(|e| {
                matches!(e, DomainEvent::TranscriptSuggestionsChanged { coaching_session_id, target, .. }
                    if *coaching_session_id == session_id && target == "action")
            })
            .count()
    }
}

/// A sweep over `provider` whose events are recorded.
fn recorded(provider: Arc<dyn analysis::Provider>) -> (Sweep, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let sweep = Sweep {
        provider,
        publisher: EventPublisher::new().with_handler(recorder.clone()),
    };
    (sweep, recorder)
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

#[tokio::test]
async fn a_transient_database_error_on_the_first_attempt_is_released_for_retry() {
    within_time_limit(async {
        let db = database().await;
        let (id, _) = seed_pending(&db, 0).await;
        // Loading the session's actions then fails with a database error, not a missing record.
        db.execute_unprepared("DROP TABLE refactor_platform.actions")
            .await
            .expect("the actions table is dropped");
        let ctx = context(db);

        sweep(None).run(&ctx).await.expect("the tick runs");

        let row = reread(&ctx, id).await;
        assert_eq!(row.status, Status::Pending);
        assert_eq!(row.claimed_at, None);
        assert_eq!(row.attempts, 1);
        assert!(row.error_message.is_some());
    })
    .await;
}

#[tokio::test]
async fn a_run_with_no_suggestions_announces_once() {
    within_time_limit(async {
        let db = database().await;
        let (id, session_id) = seed_pending(&db, 0).await;
        let ctx = context(db);
        let (sweep, recorder) = recorded(Arc::new(FakeModel { fail: None }));

        sweep.run(&ctx).await.expect("the tick runs");

        assert_eq!(reread(&ctx, id).await.status, Status::Completed);
        assert_eq!(recorder.action_changes(session_id), 1);
    })
    .await;
}

#[tokio::test]
async fn a_run_that_gives_up_announces_once() {
    within_time_limit(async {
        let db = database().await;
        let (id, session_id) = seed_pending(&db, 2).await;
        let ctx = context(db);
        let (sweep, recorder) = recorded(Arc::new(FakeModel {
            fail: Some(network),
        }));

        sweep.run(&ctx).await.expect("the tick runs");

        assert_eq!(reread(&ctx, id).await.status, Status::Failed);
        assert_eq!(recorder.action_changes(session_id), 1);
    })
    .await;
}

#[tokio::test]
async fn a_run_released_for_retry_announces_nothing() {
    within_time_limit(async {
        let db = database().await;
        let (id, session_id) = seed_pending(&db, 0).await;
        let ctx = context(db);
        let (sweep, recorder) = recorded(Arc::new(FakeModel {
            fail: Some(network),
        }));

        sweep.run(&ctx).await.expect("the tick runs");

        assert_eq!(reread(&ctx, id).await.status, Status::Pending);
        assert_eq!(recorder.action_changes(session_id), 0);
    })
    .await;
}

/// Another worker retakes and completes the analysis during the call, which then fails late.
struct RetakenThenFails {
    db: Arc<DatabaseConnection>,
    id: Id,
    retaken_attempts: i16,
}

#[async_trait]
impl analysis::Provider for RetakenThenFails {
    async fn complete(&self, _request: Request) -> Result<Completion, meeting_ai::Error> {
        transcript_analysis::ActiveModel {
            id: Set(self.id),
            status: Set(Status::Completed),
            attempts: Set(self.retaken_attempts),
            ..Default::default()
        }
        .update(self.db.as_ref())
        .await
        .expect("the other worker completes it");
        Err(network())
    }

    fn provider_id(&self) -> &str {
        "fake"
    }
}

#[tokio::test]
async fn a_late_failure_never_undoes_another_workers_completion() {
    within_time_limit(async {
        let db = database().await;
        // First attempt (released on failure) and last attempt (failed on failure).
        for (seeded, retaken_attempts) in [(0, 2), (2, 3)] {
            let (id, session_id) = seed_pending(&db, seeded).await;
            let ctx = context(db.clone());
            let (sweep, recorder) = recorded(Arc::new(RetakenThenFails {
                db: ctx.db.clone(),
                id,
                retaken_attempts,
            }));

            sweep.run(&ctx).await.expect("the tick runs");

            let row = reread(&ctx, id).await;
            assert_eq!(row.status, Status::Completed, "seeded at {seeded}");
            assert_eq!(row.attempts, retaken_attempts);
            assert_eq!(row.error_message, None);
            assert_eq!(recorder.action_changes(session_id), 0);
        }
    })
    .await;
}

/// Answers with no suggestions and records when each call began.
#[derive(Default)]
struct RecordsCalls(Mutex<Vec<chrono::DateTime<chrono::FixedOffset>>>);

#[async_trait]
impl analysis::Provider for RecordsCalls {
    async fn complete(&self, _request: Request) -> Result<Completion, meeting_ai::Error> {
        self.0
            .lock()
            .expect("the lock is free")
            .push(Utc::now().fixed_offset());
        Ok(Completion {
            output: json!({ "suggestions": [] }),
            model: "fake-model".to_string(),
            usage: Usage {
                input_tokens: 10,
                output_tokens: 2,
            },
            request_id: None,
        })
    }

    fn provider_id(&self) -> &str {
        "fake"
    }
}

#[tokio::test]
async fn each_claim_is_stamped_when_it_is_made_not_when_the_tick_began() {
    within_time_limit(async {
        let db = database().await;
        let (first, _) = seed_pending(&db, 0).await;
        let (second, _) = seed_pending(&db, 0).await;
        let ctx = context(db);
        let calls = Arc::new(RecordsCalls::default());
        let (sweep, _) = recorded(calls.clone());

        sweep.run(&ctx).await.expect("the tick runs");

        let first_call = calls.0.lock().expect("the lock is free")[0];
        let claimed = [reread(&ctx, first).await, reread(&ctx, second).await]
            .map(|row| row.claimed_at.expect("the row was claimed"));
        let last_claim = claimed.iter().max().expect("two claims");
        assert!(
            *last_claim >= first_call,
            "the later claim ({last_claim}) predates the first model call ({first_call})"
        );
    })
    .await;
}

#[tokio::test]
async fn a_run_that_stores_suggestions_announces_once() {
    within_time_limit(async {
        let db = database().await;
        let (id, session_id) = seed_pending(&db, 0).await;
        let ctx = context(db);
        let (sweep, recorder) = recorded(Arc::new(OneSuggestion));

        sweep.run(&ctx).await.expect("the tick runs");

        assert_eq!(reread(&ctx, id).await.status, Status::Completed);
        let stored = entity::transcript_suggestion::Entity::find()
            .all(ctx.db.as_ref())
            .await
            .expect("the suggestions are read");
        assert_eq!(stored.len(), 1);
        assert_eq!(recorder.action_changes(session_id), 1);
    })
    .await;
}

#[test]
fn only_bad_credentials_and_missing_records_are_permanent() {
    let domain = |kind| {
        RunError::Domain(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(kind),
        })
    };

    assert!(!should_retry(&RunError::Provider(authentication())));
    assert!(!should_retry(&RunError::Provider(
        meeting_ai::Error::Configuration("bad base url".to_string())
    )));
    assert!(should_retry(&RunError::Provider(network())));
    assert!(should_retry(&RunError::Verification(Error {
        source: None,
        error_kind: DomainErrorKind::Validation("no suggestions array".to_string()),
    })));
    assert!(!should_retry(&domain(InternalErrorKind::Entity(
        EntityErrorKind::NotFound
    ))));
    assert!(should_retry(&domain(InternalErrorKind::Other(
        "pool timed out".to_string()
    ))));
}
