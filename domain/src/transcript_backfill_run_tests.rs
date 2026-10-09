//! A whole transcription through the backfill: classification order, dry run versus apply, and
//! the option parsing that guards the run.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use entity::meeting_recording::{self, MeetingRecordingStatus};
use entity::transcription::TranscriptionStatus;
use entity::{coaching_relationships, coaching_sessions, transcript_segment};
use meeting_ai::types::recording::{Config as BotConfig, Filters, Info, Status as BotStatus};
use meeting_ai::types::transcription::{
    Config as TranscriptConfig, Participant as SpeakerIn, Segment as Rebuilt, Status, Transcription,
};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};

use super::*;
use crate::meeting_provider::Provider as MeetingProvider;
use crate::test_utils::sqlite::{database, seed_organization, seed_user, within_time_limit};
use crate::transcript_participant as participant_api;

const MEET_URL: &str = "https://meet.google.com/abc-mnop-xyz";
const MEET_CODE: &str = "abc-mnop-xyz";

struct FakeBot;

#[async_trait]
impl recording_bot::Provider for FakeBot {
    async fn create_bot(&self, _: BotConfig) -> Result<Info, meeting_ai::Error> {
        Err(meeting_ai::Error::Provider("unused".into()))
    }

    async fn get_bot_status(&self, bot_id: &str) -> Result<Info, meeting_ai::Error> {
        Ok(Info {
            id: bot_id.to_string(),
            meeting_url: String::new(),
            meeting_id: Some(MEET_CODE.to_string()),
            status: BotStatus::Completed,
            artifacts: None,
            error_message: None,
            status_history: vec![],
        })
    }

    async fn stop_bot(&self, _: &str) -> Result<(), meeting_ai::Error> {
        Ok(())
    }

    async fn list_bots(&self, _: Option<Filters>) -> Result<Vec<Info>, meeting_ai::Error> {
        Ok(vec![])
    }

    fn provider_id(&self) -> &str {
        "fake_bot"
    }
}

/// Returns the given transcript for any id and counts the calls.
struct FakeTranscripts {
    transcript: Transcription,
    calls: AtomicUsize,
}

#[async_trait]
impl transcription_trait::Provider for FakeTranscripts {
    async fn create_transcription(
        &self,
        _: TranscriptConfig,
    ) -> Result<Transcription, meeting_ai::Error> {
        Err(meeting_ai::Error::Provider("unused".into()))
    }

    async fn get_transcription(&self, _: &str) -> Result<Transcription, meeting_ai::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.transcript.clone())
    }

    async fn delete_transcription(&self, _: &str) -> Result<(), meeting_ai::Error> {
        Ok(())
    }

    fn provider_id(&self) -> &str {
        "fake_transcripts"
    }
}

fn transcripts(segments: Vec<Rebuilt>) -> FakeTranscripts {
    let speaker = |id: &str, is_host| SpeakerIn {
        provider_id: id.to_string(),
        display_name: Some(format!("Typed {id}")),
        is_host: Some(is_host),
        platform: None,
        account_id: None,
        extra_data: None,
    };
    FakeTranscripts {
        transcript: Transcription {
            id: "ext".to_string(),
            status: Status::Completed,
            text: None,
            words: vec![],
            segments,
            participants: vec![speaker("100", true), speaker("200", false)],
            chapters: vec![],
            sentiment_analysis: vec![],
            confidence: None,
            duration_seconds: None,
            language_code: None,
            speaker_count: None,
            error_message: None,
        },
        calls: AtomicUsize::new(0),
    }
}

/// Like `transcripts`, but nobody hosts, so without Google nobody can be identified.
fn hostless_transcripts(segments: Vec<Rebuilt>) -> FakeTranscripts {
    let mut recall = transcripts(segments);
    recall
        .transcript
        .participants
        .iter_mut()
        .for_each(|p| p.is_host = Some(false));
    recall
}

fn rebuilt(participant: &str, start_ms: i64, text: &str) -> Rebuilt {
    Rebuilt {
        text: text.to_string(),
        speaker: "Typed".to_string(),
        participant_id: Some(participant.to_string()),
        start_ms,
        end_ms: start_ms + 1000,
        confidence: 0.0,
        words: vec![],
    }
}

/// The two segments every fixture stores.
fn matching() -> Vec<Rebuilt> {
    vec![
        rebuilt("100", 0, "Good morning."),
        rebuilt("200", 4000, "Morning."),
    ]
}

/// A completed legacy transcription in a session with the given meeting link.
async fn seed(db: &DatabaseConnection, meeting_url: Option<&str>) -> transcription::Model {
    let now = Utc::now();
    let relationship_id = Id::new_v4();
    coaching_relationships::ActiveModel {
        id: Set(relationship_id),
        organization_id: Set(seed_organization(db).await),
        coach_id: Set(seed_user(db).await),
        coachee_id: Set(seed_user(db).await),
        slug: Set(format!("relationship-{relationship_id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
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
        meeting_url: Set(meeting_url.map(str::to_string)),
        provider: Set(Some(MeetingProvider::Google)),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        hydrated_at: Set(None),
        notice_given_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the session is seeded");

    let recording = meeting_recording::ActiveModel {
        id: Set(Id::new_v4()),
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
    .insert(db)
    .await
    .expect("the recording is seeded");

    let transcription = transcription::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session.id),
        meeting_recording_id: Set(recording.id),
        external_id: Set("ext".to_string()),
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

    for (start_ms, text) in [(0, "Good morning."), (4000, "Morning.")] {
        transcript_segment::ActiveModel {
            id: Set(Id::new_v4()),
            transcription_id: Set(transcription.id),
            participant_id: Set(None),
            speaker_label: Set("Typed".to_string()),
            text: Set(text.to_string()),
            start_ms: Set(start_ms),
            end_ms: Set(start_ms + 1000),
            confidence: Set(None),
            sentiment: Set(None),
            created_at: Set(now.into()),
        }
        .insert(db)
        .await
        .expect("the segment is seeded");
    }

    transcription
}

fn options(apply: bool) -> Options {
    Options {
        apply,
        google: false,
        limit: None,
        since: None,
        delay: Duration::ZERO,
    }
}

async fn links(db: &DatabaseConnection, transcription_id: Id) -> Vec<Option<Id>> {
    entity_api::transcript_segment::find_by_transcription(db, transcription_id)
        .await
        .expect("segments read")
        .into_iter()
        .map(|s| s.participant_id)
        .collect()
}

async fn run(
    db: &DatabaseConnection,
    transcripts: &FakeTranscripts,
    transcription: &transcription::Model,
    apply: bool,
) -> Row {
    let providers = Providers {
        transcripts,
        bot: &FakeBot,
    };
    process(
        db,
        &providers,
        &Config::default(),
        transcription,
        &options(apply),
    )
    .await
}

#[tokio::test]
async fn a_dry_run_identifies_the_speakers_but_writes_nothing() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription = seed(&db, Some(MEET_URL)).await;

        let row = run(&db, &transcripts(matching()), &transcription, false).await;

        assert_eq!(
            (row.outcome, row.speakers, row.coach, row.coachee),
            (Outcome::WouldAttribute, Some(2), true, true)
        );
        assert!(
            participant_api::find_by_transcription(&db, transcription.id)
                .await?
                .is_empty()
        );
        assert_eq!(links(&db, transcription.id).await, vec![None, None]);
        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn applying_links_every_stored_segment() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription = seed(&db, Some(MEET_URL)).await;

        let row = run(&db, &transcripts(matching()), &transcription, true).await;

        assert_eq!(row.outcome, Outcome::Attributed);
        assert_eq!(
            participant_api::find_by_transcription(&db, transcription.id)
                .await?
                .len(),
            2
        );
        assert!(links(&db, transcription.id)
            .await
            .iter()
            .all(Option::is_some));
        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn a_session_without_a_meet_link_never_reaches_recall() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription = seed(&db, None).await;
        let recall = transcripts(matching());

        let row = run(&db, &recall, &transcription, true).await;

        assert_eq!(row.outcome, Outcome::NotGoogleMeet);
        assert_eq!(recall.calls.load(Ordering::SeqCst), 0);
        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn a_rebuild_that_differs_is_left_alone() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription = seed(&db, Some(MEET_URL)).await;
        let differing = vec![rebuilt("100", 0, "Good morning.")];

        let row = run(&db, &transcripts(differing), &transcription, true).await;

        assert_eq!(row.outcome, Outcome::SegmentsDiffer);
        assert!(
            participant_api::find_by_transcription(&db, transcription.id)
                .await?
                .is_empty()
        );
        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn applying_with_nobody_identified_links_nothing_and_stays_a_candidate() -> Result<(), Error>
{
    within_time_limit(async {
        let db = database().await;
        let transcription = seed(&db, Some(MEET_URL)).await;

        let row = run(&db, &hostless_transcripts(matching()), &transcription, true).await;

        assert_eq!(
            (row.outcome, row.coach, row.coachee),
            (Outcome::NobodyIdentified, false, false)
        );
        assert!(
            participant_api::find_by_transcription(&db, transcription.id)
                .await?
                .is_empty()
        );
        assert_eq!(links(&db, transcription.id).await, vec![None, None]);
        assert!(find_candidates(&db, None, None)
            .await?
            .iter()
            .any(|t| t.id == transcription.id));
        Ok::<(), Error>(())
    })
    .await
}

fn lookup<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        vars.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.to_string())
    }
}

#[test]
fn unset_options_are_a_dry_run_with_google_off() {
    assert_eq!(
        Options::from_lookup(lookup(&[])),
        Ok(Options {
            apply: false,
            google: false,
            limit: None,
            since: None,
            delay: Duration::from_millis(300),
        })
    );
}

#[test]
fn google_without_apply_is_rejected() {
    let parsed = Options::from_lookup(lookup(&[("BACKFILL_GOOGLE", "1")]));
    assert!(parsed.is_err_and(|e| e.contains("BACKFILL_APPLY=1")));
}

#[test]
fn google_with_apply_is_accepted() {
    let parsed = Options::from_lookup(lookup(&[("BACKFILL_GOOGLE", "1"), ("BACKFILL_APPLY", "1")]));
    assert!(parsed.is_ok_and(|o| o.google && o.apply));
}

#[test]
fn an_invalid_since_is_rejected() {
    for value in ["2026-13-01", "yesterday", "2026-09-01T00:00:00Z", ""] {
        let parsed = Options::from_lookup(lookup(&[("BACKFILL_SINCE", value)]));
        assert!(
            parsed.is_err_and(|e| e.contains("BACKFILL_SINCE")),
            "{value:?}"
        );
    }
}

#[test]
fn since_is_midnight_utc() {
    assert_eq!(
        parse_since("2026-09-01"),
        Ok(Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap())
    );
}

#[test]
fn other_invalid_options_are_rejected() {
    for vars in [
        [("BACKFILL_APPLY", "yes")],
        [("BACKFILL_GOOGLE", "off")],
        [("BACKFILL_LIMIT", "0")],
        [("BACKFILL_LIMIT", "-1")],
        [("BACKFILL_DELAY_MS", "fast")],
    ] {
        assert!(Options::from_lookup(lookup(&vars)).is_err(), "{vars:?}");
    }
}

#[test]
fn a_report_line_never_breaks_its_columns() {
    let row = Row {
        transcription_id: Id::nil(),
        coaching_session_id: Id::nil(),
        outcome: Outcome::Error,
        speakers: None,
        coach: false,
        coachee: true,
        detail: Some("one, two".to_string()),
    };

    assert_eq!(
        row.csv_line().split(',').count(),
        CSV_HEADER.split(',').count()
    );
    assert!(row.csv_line().ends_with(",error,,no,yes,one; two"));
}

/// Stores a segment with a chosen id, so the tie order among equal starts is known.
async fn store_segment(
    db: &DatabaseConnection,
    transcription_id: Id,
    id: Id,
    start_ms: i32,
    text: &str,
) {
    let now = Utc::now();
    transcript_segment::ActiveModel {
        id: Set(id),
        transcription_id: Set(transcription_id),
        participant_id: Set(None),
        speaker_label: Set("Typed".to_string()),
        text: Set(text.to_string()),
        start_ms: Set(start_ms),
        end_ms: Set(start_ms + 1000),
        confidence: Set(None),
        sentiment: Set(None),
        created_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the segment is seeded");
}

#[tokio::test]
async fn a_tie_stored_in_the_other_order_links_each_line_to_its_speaker() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription = seed(&db, Some(MEET_URL)).await;
        // Stored order breaks the tie by id: "Yes." (200) before "Right, so" (100).
        let yes = Id::from_u128(1);
        let right_so = Id::from_u128(2);
        store_segment(&db, transcription.id, yes, 2000, "Yes.").await;
        store_segment(&db, transcription.id, right_so, 2000, "Right, so").await;
        let rebuild = vec![
            rebuilt("100", 0, "Good morning."),
            rebuilt("100", 2000, "Right, so"),
            rebuilt("200", 2000, "Yes."),
            rebuilt("200", 4000, "Morning."),
        ];

        let row = run(&db, &transcripts(rebuild), &transcription, true).await;

        assert_eq!(row.outcome, Outcome::Attributed);
        let row_of: HashMap<String, Id> =
            participant_api::find_by_transcription(&db, transcription.id)
                .await?
                .into_iter()
                .map(|p| (p.provider_participant_id, p.id))
                .collect();
        let linked: HashMap<Id, Option<Id>> =
            entity_api::transcript_segment::find_by_transcription(&db, transcription.id)
                .await?
                .into_iter()
                .map(|s| (s.id, s.participant_id))
                .collect();
        assert_eq!(linked.get(&yes), Some(&row_of.get("200").copied()));
        assert_eq!(linked.get(&right_so), Some(&row_of.get("100").copied()));
        Ok::<(), Error>(())
    })
    .await
}
