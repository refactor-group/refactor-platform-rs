//! Reviewing action suggestions against a real SQL engine: what each operation leaves in
//! `actions`, `actions_users` and `transcript_suggestions`, and what it announces.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{NaiveDateTime, TimeZone};
use entity::meeting_recording::{self, MeetingRecordingStatus};
use entity::transcription::TranscriptionStatus;
use entity::{actions_users, coaching_relationships, transcript_segment, transcript_suggestion};
use meeting_ai::types::transcription::{
    Participant as SpeakerIn, Segment as SegmentIn, Status as TranscriptStatus, Transcription,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, ModelTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set,
};

use super::*;
use crate::events::{DomainEvent, EventHandler};
use crate::test_utils::sqlite::{database, seed_organization, within_time_limit};
use crate::transcript_attribution::Attribution;
use crate::transcript_participant::MatchSource;
use crate::transcription::persist_completion;

const COACHEE_NAME: &str = "Casey Coachee";

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
    fn suggestion_changes(&self) -> usize {
        self.0
            .lock()
            .expect("the lock is free")
            .iter()
            .filter(|e| matches!(e, DomainEvent::TranscriptSuggestionsChanged { target, .. } if target == "action"))
            .count()
    }
}

fn recording_publisher() -> (EventPublisher, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    (
        EventPublisher::new().with_handler(recorder.clone()),
        recorder,
    )
}

/// 2026-10-12 15:00 UTC, the fixed "now" of every test.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 15, 0, 0)
        .single()
        .expect("valid time")
}

/// The reviewed session starts an hour before `now`.
fn session_start() -> NaiveDateTime {
    (now() - Duration::hours(1)).naive_utc()
}

struct Fixture {
    db: DatabaseConnection,
    coach: users::Model,
    coachee: users::Model,
    relationship_id: Id,
    session: coaching_sessions::Model,
    analysis_id: Id,
    /// Segment ids in speaking order: coach, coachee, coach.
    segment_ids: Vec<Id>,
}

async fn seed_person(db: &DatabaseConnection, name: &str) -> users::Model {
    let id = Id::new_v4();
    let at = Utc::now();
    users::ActiveModel {
        id: Set(id),
        email: Set(format!("{id}@example.com")),
        first_name: Set("Test".to_string()),
        last_name: Set("User".to_string()),
        display_name: Set(Some(name.to_string())),
        password: Set(None),
        github_username: Set(None),
        github_profile_url: Set(None),
        timezone: Set("UTC".to_string()),
        default_coaching_session_duration_minutes: Set(60),
        created_at: Set(at.into()),
        updated_at: Set(at.into()),
    }
    .insert(db)
    .await
    .expect("the user is seeded")
}

async fn seed_session(
    db: &DatabaseConnection,
    relationship_id: Id,
    date: NaiveDateTime,
) -> coaching_sessions::Model {
    let at = Utc::now();
    coaching_sessions::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_relationship_id: Set(relationship_id),
        coaching_session_series_id: Set(None),
        ical_sequence: Set(0),
        ical_recurrence_id: Set(None),
        collab_document_name: Set(None),
        date: Set(date),
        duration_minutes: Set(60),
        title: Set(None),
        meeting_url: Set(None),
        provider: Set(None),
        created_at: Set(at.into()),
        updated_at: Set(at.into()),
        hydrated_at: Set(None),
        notice_given_at: Set(at.into()),
    }
    .insert(db)
    .await
    .expect("the session is seeded")
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

fn spoken(provider_id: &str, text: &str, start_ms: i64) -> SegmentIn {
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

/// A coach and coachee, their session with a completed three-segment transcript, and an analysis.
async fn fixture_in(db: DatabaseConnection) -> Fixture {
    let coach = seed_person(&db, "Jordan Coach").await;
    let coachee = seed_person(&db, COACHEE_NAME).await;
    let at = Utc::now();

    let relationship_id = Id::new_v4();
    coaching_relationships::ActiveModel {
        id: Set(relationship_id),
        organization_id: Set(seed_organization(&db).await),
        coach_id: Set(coach.id),
        coachee_id: Set(coachee.id),
        slug: Set(format!("relationship-{relationship_id}")),
        created_at: Set(at.into()),
        updated_at: Set(at.into()),
    }
    .insert(&db)
    .await
    .expect("the relationship is seeded");
    let session = seed_session(&db, relationship_id, session_start()).await;

    let recording_id = Id::new_v4();
    meeting_recording::ActiveModel {
        id: Set(recording_id),
        coaching_session_id: Set(session.id),
        bot_id: Set(format!("bot-{recording_id}")),
        status: Set(MeetingRecordingStatus::Completed),
        video_url: Set(None),
        audio_url: Set(None),
        duration_seconds: Set(None),
        started_at: Set(None),
        ended_at: Set(None),
        error_message: Set(None),
        created_at: Set(at.into()),
        updated_at: Set(at.into()),
    }
    .insert(&db)
    .await
    .expect("the recording is seeded");

    let external_id = format!("ext-{recording_id}");
    let transcription = entity::transcription::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session.id),
        meeting_recording_id: Set(recording_id),
        external_id: Set(external_id.clone()),
        recall_recording_id: Set(None),
        status: Set(TranscriptionStatus::Processing),
        language_code: Set(None),
        speaker_count: Set(None),
        word_count: Set(None),
        duration_seconds: Set(None),
        confidence: Set(None),
        error_message: Set(None),
        created_at: Set(at.into()),
        updated_at: Set(at.into()),
    }
    .insert(&db)
    .await
    .expect("the transcription is seeded");

    let result = Transcription {
        id: external_id,
        status: TranscriptStatus::Completed,
        text: None,
        words: vec![],
        segments: vec![
            spoken("100", "What will you do before we next meet?", 0),
            spoken("200", "I will send you the org chart by Friday.", 4000),
            spoken("100", "And I will share the article.", 9000),
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
        [("100", coach.id), ("200", coachee.id)].map(|(provider_id, user_id)| Attribution {
            provider_id: provider_id.to_string(),
            user_id: Some(user_id),
            source: Some(MatchSource::Account),
        });
    persist_completion(&db, &transcription, &result, &attributions)
        .await
        .expect("the transcript is completed");

    let segment_ids = transcript_segment::Entity::find()
        .filter(transcript_segment::Column::TranscriptionId.eq(transcription.id))
        .order_by_asc(transcript_segment::Column::StartMs)
        .all(&db)
        .await
        .expect("the segments are read")
        .into_iter()
        .map(|s| s.id)
        .collect();
    let analysis_id = transcript_analysis::create_pending(&db, transcription.id, Target::Action)
        .await
        .expect("the analysis is seeded")
        .id;

    Fixture {
        db,
        coach,
        coachee,
        relationship_id,
        session,
        analysis_id,
        segment_ids,
    }
}

async fn fixture() -> Fixture {
    fixture_in(database().await).await
}

fn proposal(
    f: &Fixture,
    body: &str,
    assignee_ids: Vec<Id>,
    due_by: Option<DateTimeWithTimeZone>,
    due_phrase: Option<&str>,
) -> ActionProposal {
    let placeholder = DateTime::<Utc>::UNIX_EPOCH.fixed_offset();
    ActionProposal {
        action: ActionWithAssignees {
            action: actions::Model {
                id: Id::nil(),
                coaching_session_id: f.session.id,
                goal_id: None,
                user_id: Id::nil(),
                body: Some(body.to_string()),
                due_by,
                status: ActionStatus::NotStarted,
                status_changed_at: placeholder,
                created_at: placeholder,
                updated_at: placeholder,
            },
            assignee_ids,
        },
        due_phrase: due_phrase.map(str::to_string),
    }
}

/// The edits that turn a suggestion into `proposal`.
fn edits_of(proposal: ActionProposal) -> ActionEdits {
    ActionEdits {
        body: proposal.action.action.body.unwrap_or_default(),
        due_by: proposal.action.action.due_by,
        assignee_ids: proposal.action.assignee_ids,
    }
}

/// A suggestion of `proposal` from the coachee's segment.
async fn suggest(
    f: &Fixture,
    proposal: &ActionProposal,
    status: Status,
    duplicate_of_id: Option<Id>,
) -> Model {
    transcript_suggestion::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(f.session.id),
        transcript_analysis_id: Set(f.analysis_id),
        target: Set(Target::Action),
        proposal: Set(serde_json::to_value(proposal).expect("the proposal serializes")),
        segment_id: Set(f.segment_ids[1]),
        quote: Set("send you the org chart".to_string()),
        duplicate_of_id: Set(duplicate_of_id),
        status: Set(status),
        resolved_by: Set(None),
        resolved_at: Set(None),
        created_entity_id: Set(None),
        created_at: Set(now().fixed_offset()),
        updated_at: Set(now().fixed_offset()),
    }
    .insert(&f.db)
    .await
    .expect("the suggestion is seeded")
}

async fn reread(f: &Fixture, id: Id) -> Model {
    transcript_suggestion::Entity::find_by_id(id)
        .one(&f.db)
        .await
        .expect("the suggestion is read")
        .expect("the suggestion exists")
}

async fn session_actions(f: &Fixture) -> Vec<actions::Model> {
    actions::Entity::find()
        .filter(actions::Column::CoachingSessionId.eq(f.session.id))
        .all(&f.db)
        .await
        .expect("the actions are read")
}

async fn assignees_of(f: &Fixture, action_id: Id) -> Vec<Id> {
    actions_users::Entity::find()
        .filter(actions_users::Column::ActionId.eq(action_id))
        .all(&f.db)
        .await
        .expect("the assignees are read")
        .into_iter()
        .map(|a| a.user_id)
        .collect()
}

fn due(days: i64) -> DateTimeWithTimeZone {
    (now() + Duration::days(days)).fixed_offset()
}

fn kind(error: &Error) -> &DomainErrorKind {
    &error.error_kind
}

fn is_forbidden(error: &Error) -> bool {
    matches!(
        kind(error),
        DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Forbidden))
    )
}

fn is_conflict(error: &Error) -> bool {
    matches!(
        kind(error),
        DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Conflict { .. }))
    )
}

fn is_validation(error: &Error) -> bool {
    matches!(kind(error), DomainErrorKind::Validation(_))
}

#[tokio::test]
async fn accept_creates_exactly_one_action_owned_by_the_accepting_user() {
    within_time_limit(async {
        let f = fixture().await;
        let (publisher, recorder) = recording_publisher();
        let p = proposal(
            &f,
            "Send the org chart",
            vec![f.coachee.id],
            Some(due(3)),
            Some("by Friday"),
        );
        let s = suggest(&f, &p, Status::Pending, None).await;

        let created = accept_action(
            &f.db,
            &publisher,
            &Config::default(),
            &f.coachee,
            s.clone(),
            None,
            now(),
        )
        .await
        .expect("the coachee accepts");

        let rows = session_actions(&f).await;
        assert_eq!(rows.len(), 1);
        let action = &rows[0];
        assert_eq!(action.id, created.action.id);
        assert_eq!(action.body.as_deref(), Some("Send the org chart"));
        assert_eq!(action.due_by, Some(due(3)));
        assert_eq!(action.user_id, f.coachee.id);
        assert_eq!(action.coaching_session_id, f.session.id);
        assert_eq!(action.status, ActionStatus::NotStarted);
        assert_eq!(assignees_of(&f, action.id).await, vec![f.coachee.id]);

        let stored = reread(&f, s.id).await;
        assert_eq!(stored.status, Status::Accepted);
        assert_eq!(stored.resolved_by, Some(f.coachee.id));
        assert_eq!(stored.created_entity_id, Some(action.id));
        assert_eq!(recorder.suggestion_changes(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_second_accept_conflicts_and_creates_nothing() {
    within_time_limit(async {
        let f = fixture().await;
        let (publisher, recorder) = recording_publisher();
        let p = proposal(
            &f,
            "Send the org chart",
            vec![f.coachee.id],
            Some(due(3)),
            None,
        );
        let s = suggest(&f, &p, Status::Pending, None).await;

        accept_action(
            &f.db,
            &publisher,
            &Config::default(),
            &f.coachee,
            s.clone(),
            None,
            now(),
        )
        .await
        .expect("the first accept succeeds");
        // The stale pending model stands in for a racing second request.
        let second = accept_action(
            &f.db,
            &publisher,
            &Config::default(),
            &f.coach,
            s,
            None,
            now(),
        )
        .await
        .expect_err("the second accept is refused");

        assert!(is_conflict(&second), "{second:?}");
        assert_eq!(session_actions(&f).await.len(), 1);
        let assigned = actions_users::Entity::find()
            .count(&f.db)
            .await
            .expect("the assignees are counted");
        assert_eq!(assigned, 1);
        assert_eq!(recorder.suggestion_changes(), 1);
    })
    .await;
}

#[tokio::test]
async fn accept_without_a_due_date_uses_the_next_session_start() {
    within_time_limit(async {
        let f = fixture().await;
        let next = seed_session(
            &f.db,
            f.relationship_id,
            (now() + Duration::days(3)).naive_utc(),
        )
        .await;
        let (publisher, _) = recording_publisher();
        let s = suggest(
            &f,
            &proposal(&f, "Read the article", vec![], None, None),
            Status::Pending,
            None,
        )
        .await;

        accept_action(
            &f.db,
            &publisher,
            &Config::default(),
            &f.coach,
            s,
            None,
            now(),
        )
        .await
        .expect("the coach accepts");

        let rows = session_actions(&f).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].due_by, Some(next.date.and_utc().fixed_offset()));
    })
    .await;
}

#[tokio::test]
async fn accept_without_a_due_date_or_a_next_session_is_due_in_seven_days() {
    within_time_limit(async {
        let f = fixture().await;
        let (publisher, _) = recording_publisher();
        let s = suggest(
            &f,
            &proposal(&f, "Read the article", vec![], None, None),
            Status::Pending,
            None,
        )
        .await;

        accept_action(
            &f.db,
            &publisher,
            &Config::default(),
            &f.coach,
            s,
            None,
            now(),
        )
        .await
        .expect("the coach accepts");

        let rows = session_actions(&f).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].due_by, Some(due(7)));
    })
    .await;
}

#[tokio::test]
async fn accept_with_edits_uses_them_and_rejects_an_outside_assignee() {
    within_time_limit(async {
        let f = fixture().await;
        let (publisher, recorder) = recording_publisher();
        let p = proposal(
            &f,
            "Send the org chart",
            vec![f.coachee.id],
            Some(due(3)),
            None,
        );
        let outsider = suggest(&f, &p, Status::Pending, None).await;

        let bad = proposal(
            &f,
            "Send the org chart",
            vec![Id::new_v4()],
            Some(due(3)),
            None,
        );
        let refused = accept_action(
            &f.db,
            &publisher,
            &Config::default(),
            &f.coach,
            outsider.clone(),
            Some(edits_of(bad)),
            now(),
        )
        .await
        .expect_err("an outside assignee is refused");
        assert!(is_validation(&refused), "{refused:?}");
        assert!(session_actions(&f).await.is_empty());
        assert_eq!(reread(&f, outsider.id).await.status, Status::Pending);
        assert_eq!(recorder.suggestion_changes(), 0);

        let edits = proposal(
            &f,
            "Send the revised org chart",
            vec![f.coachee.id],
            Some(due(5)),
            None,
        );
        accept_action(
            &f.db,
            &publisher,
            &Config::default(),
            &f.coach,
            outsider,
            Some(edits_of(edits)),
            now(),
        )
        .await
        .expect("the edited accept succeeds");

        let rows = session_actions(&f).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].body.as_deref(), Some("Send the revised org chart"));
        assert_eq!(rows[0].due_by, Some(due(5)));
        assert_eq!(rows[0].user_id, f.coach.id);
    })
    .await;
}

#[tokio::test]
async fn the_coachee_may_not_accept_a_coach_only_suggestion_or_add_the_coach() {
    within_time_limit(async {
        let f = fixture().await;
        let (publisher, recorder) = recording_publisher();
        let coach_only = suggest(
            &f,
            &proposal(&f, "Share the article", vec![f.coach.id], None, None),
            Status::Pending,
            None,
        )
        .await;

        let refused = accept_action(
            &f.db,
            &publisher,
            &Config::default(),
            &f.coachee,
            coach_only.clone(),
            None,
            now(),
        )
        .await
        .expect_err("a coach-only accept is refused");
        assert!(is_forbidden(&refused), "{refused:?}");
        assert!(session_actions(&f).await.is_empty());
        assert_eq!(reread(&f, coach_only.id).await.status, Status::Pending);

        let p = proposal(&f, "Send the org chart", vec![f.coachee.id], None, None);
        let coachee_only = suggest(&f, &p, Status::Pending, None).await;
        let adds_coach = proposal(
            &f,
            "Send the org chart",
            vec![f.coachee.id, f.coach.id],
            None,
            None,
        );
        let refused = edit_action(
            &f.db,
            &publisher,
            &f.coachee,
            coachee_only.clone(),
            edits_of(adds_coach),
        )
        .await
        .expect_err("adding the coach is refused");
        assert!(is_forbidden(&refused), "{refused:?}");
        assert_eq!(
            reread(&f, coachee_only.id).await.proposal,
            coachee_only.proposal
        );
        assert_eq!(recorder.suggestion_changes(), 0);
    })
    .await;
}

#[tokio::test]
async fn edit_rewrites_the_proposal_and_a_new_due_date_drops_the_phrase() {
    within_time_limit(async {
        let f = fixture().await;
        let (publisher, recorder) = recording_publisher();
        let p = proposal(
            &f,
            "Send the org chart",
            vec![f.coachee.id],
            Some(due(3)),
            Some("by Friday"),
        );
        let s = suggest(&f, &p, Status::Pending, None).await;

        let same_due = proposal(
            &f,
            "Send the full org chart",
            vec![f.coachee.id],
            Some(due(3)),
            None,
        );
        edit_action(&f.db, &publisher, &f.coachee, s.clone(), edits_of(same_due))
            .await
            .expect("the edit succeeds");
        let stored: ActionProposal =
            serde_json::from_value(reread(&f, s.id).await.proposal).expect("the proposal reads");
        assert_eq!(
            stored.action.action.body.as_deref(),
            Some("Send the full org chart")
        );
        assert_eq!(stored.due_phrase.as_deref(), Some("by Friday"));

        let new_due = proposal(
            &f,
            "Send the full org chart",
            vec![f.coachee.id],
            Some(due(4)),
            Some("by Friday"),
        );
        edit_action(&f.db, &publisher, &f.coachee, s.clone(), edits_of(new_due))
            .await
            .expect("the edit succeeds");
        let stored: ActionProposal =
            serde_json::from_value(reread(&f, s.id).await.proposal).expect("the proposal reads");
        assert_eq!(stored.action.action.due_by, Some(due(4)));
        assert_eq!(stored.due_phrase, None);
        assert_eq!(stored.action.action.coaching_session_id, f.session.id);
        assert_eq!(recorder.suggestion_changes(), 2);

        accept_action(
            &f.db,
            &publisher,
            &Config::default(),
            &f.coachee,
            s.clone(),
            None,
            now(),
        )
        .await
        .expect("the accept succeeds");
        let late = proposal(&f, "Too late", vec![f.coachee.id], None, None);
        let refused = edit_action(&f.db, &publisher, &f.coachee, s, edits_of(late))
            .await
            .expect_err("an accepted suggestion cannot be edited");
        assert!(is_conflict(&refused), "{refused:?}");
        assert_eq!(recorder.suggestion_changes(), 3);
    })
    .await;
}

#[tokio::test]
async fn dismiss_and_restore_move_the_suggestion_and_restoring_a_pending_one_conflicts() {
    within_time_limit(async {
        let f = fixture().await;
        let (publisher, recorder) = recording_publisher();
        let s = suggest(
            &f,
            &proposal(&f, "Send the org chart", vec![f.coachee.id], None, None),
            Status::Pending,
            None,
        )
        .await;

        let dismissed = dismiss(&f.db, &publisher, &f.coachee, s.clone(), now())
            .await
            .expect("the dismiss succeeds");
        let stored = reread(&f, s.id).await;
        assert_eq!(stored.status, Status::Dismissed);
        assert_eq!(stored.resolved_by, Some(f.coachee.id));
        assert_eq!(stored.resolved_at, Some(now().fixed_offset()));

        restore(&f.db, &publisher, &f.coach, dismissed, now())
            .await
            .expect("the restore succeeds");
        let stored = reread(&f, s.id).await;
        assert_eq!(stored.status, Status::Pending);
        assert_eq!(stored.resolved_by, None);
        assert_eq!(stored.resolved_at, None);
        assert_eq!(stored.created_entity_id, None);

        let refused = restore(&f.db, &publisher, &f.coach, stored, now())
            .await
            .expect_err("a pending suggestion cannot be restored");
        assert!(is_conflict(&refused), "{refused:?}");
        assert_eq!(recorder.suggestion_changes(), 2);
    })
    .await;
}

#[tokio::test]
async fn list_returns_the_sessions_pending_suggestions_with_their_segment_and_duplicate() {
    within_time_limit(async {
        let f = fixture().await;
        let next = seed_session(
            &f.db,
            f.relationship_id,
            (now() + Duration::days(3)).naive_utc(),
        )
        .await;
        let at = Utc::now().fixed_offset();
        let existing = actions::ActiveModel {
            id: Set(Id::new_v4()),
            coaching_session_id: Set(f.session.id),
            goal_id: Set(None),
            user_id: Set(f.coach.id),
            body: Set(Some("Share the org chart".to_string())),
            due_by: Set(None),
            status: Set(ActionStatus::NotStarted),
            status_changed_at: Set(at),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&f.db)
        .await
        .expect("the action is seeded");

        let p = proposal(&f, "Send the org chart", vec![f.coachee.id], None, None);
        let flagged = suggest(&f, &p, Status::Pending, Some(existing.id)).await;
        suggest(&f, &p, Status::Dismissed, None).await;
        let other = fixture_in(f.db.clone()).await;
        suggest(
            &other,
            &proposal(&other, "Elsewhere", vec![], None, None),
            Status::Pending,
            None,
        )
        .await;

        let listed = list_pending_actions(&f.db, &f.session, now())
            .await
            .expect("the list reads");
        assert_eq!(listed.len(), 1);
        let item = &listed[0];
        assert_eq!(item.id, flagged.id);
        assert_eq!(item.coaching_session_id, f.session.id);
        assert_eq!(item.proposal, p);
        assert_eq!(item.quote, "send you the org chart");
        assert_eq!(item.segment.id, f.segment_ids[1]);
        assert_eq!(item.segment.start_ms, 4000);
        assert_eq!(item.segment.speaker_role, Some(SpeakerRole::Coachee));
        assert_eq!(item.segment.speaker_label, COACHEE_NAME);
        assert_eq!(
            item.duplicate_of,
            Some(DuplicateOf {
                action_id: existing.id,
                body: Some("Share the org chart".to_string()),
            })
        );
        assert_eq!(
            item.default_due,
            DefaultDue {
                at: next.date.and_utc().fixed_offset(),
                source: DueSource::NextSession,
            }
        );

        existing.delete(&f.db).await.expect("the action is deleted");
        let listed = list_pending_actions(&f.db, &f.session, now())
            .await
            .expect("the list reads");
        assert_eq!(listed[0].duplicate_of, None);
    })
    .await;
}

#[tokio::test]
async fn a_next_session_that_already_started_gives_seven_days() {
    within_time_limit(async {
        let f = fixture().await;
        seed_session(
            &f.db,
            f.relationship_id,
            (now() - Duration::minutes(30)).naive_utc(),
        )
        .await;

        let default = default_due(&f.db, &f.session, now())
            .await
            .expect("the default reads");

        assert_eq!(
            default,
            DefaultDue {
                at: due(7),
                source: DueSource::SevenDays,
            }
        );
    })
    .await;
}
