//! Attributes speakers on transcripts completed before attribution existed.
//!
//! The transcript is re-downloaded and rebuilt; only an exact match with the stored segments is
//! linked, and linking only inserts participant rows and fills `participant_id` on the existing
//! segments. No segment is ever inserted, deleted, or otherwise rewritten.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use entity::transcript_participant::{
    Column as ParticipantColumn, Entity as Participants, MatchSource,
};
use entity::transcript_segment::{Column as SegmentColumn, Entity as Segments};
use entity::transcription::{self, TranscriptionStatus};
use entity_api::error::Error as EntityApiError;
use entity_api::{
    coaching_session as coaching_session_api, transcript_participant as participant_api,
};
use log::*;
use meeting_ai::traits::{recording_bot, transcription as transcription_trait};
use meeting_ai::types::transcription as transcription_types;
use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{
    ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection,
    EntityName, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait, Select, Statement,
    TransactionTrait,
};
use service::config::Config;

use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use crate::transcript_attribution::Attribution;
use crate::transcription::{attribute_speakers_with, participant_rows, session_meet_code};
use crate::Id;

/// Columns of the report CSV, in order.
pub const CSV_HEADER: &str =
    "transcription_id,coaching_session_id,outcome,speakers,coach,coachee,detail";

const DEFAULT_DELAY_MS: u64 = 300;

/// A stored segment, read by explicit columns so it works before `participant_id` exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSegment {
    pub id: Id,
    pub start_ms: i32,
    pub text: String,
}

/// Whether `link_participants` wrote anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linked {
    Done,
    AlreadyDone,
}

/// How a run behaves, parsed from `BACKFILL_*` environment values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// Writes attribution; otherwise a read-only dry run.
    pub apply: bool,
    /// Consults Google's attendee lists, which needs the coach's Google token; apply mode only.
    pub google: bool,
    pub limit: Option<u64>,
    /// Only transcriptions created at or after this instant.
    pub since: Option<DateTime<Utc>>,
    /// Pause between transcriptions, to respect Recall rate limits.
    pub delay: Duration,
}

impl Options {
    /// Parses the options from `lookup`, which returns a variable's value when it is set.
    ///
    /// # Errors
    ///
    /// Returns a message naming the variable when any value is invalid, or when Google evidence
    /// is requested without apply mode.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let apply = flag("BACKFILL_APPLY", lookup("BACKFILL_APPLY"), false)?;
        let google = flag("BACKFILL_GOOGLE", lookup("BACKFILL_GOOGLE"), false)?;
        if google && !apply {
            return Err(
                "BACKFILL_GOOGLE=1 needs BACKFILL_APPLY=1: looking up Google evidence \
                 writes refreshed tokens and account ids"
                    .to_string(),
            );
        }
        Ok(Self {
            apply,
            google,
            limit: lookup("BACKFILL_LIMIT")
                .map(|v| parse_limit(&v))
                .transpose()?,
            since: lookup("BACKFILL_SINCE")
                .map(|v| parse_since(&v))
                .transpose()?,
            delay: Duration::from_millis(
                lookup("BACKFILL_DELAY_MS")
                    .map(|v| parse_u64("BACKFILL_DELAY_MS", &v))
                    .transpose()?
                    .unwrap_or(DEFAULT_DELAY_MS),
            ),
        })
    }
}

/// `1` is on and `0` is off; unset takes `default`.
fn flag(name: &str, value: Option<String>, default: bool) -> Result<bool, String> {
    match value.as_deref().map(str::trim) {
        None => Ok(default),
        Some("1") => Ok(true),
        Some("0") => Ok(false),
        Some(other) => Err(format!("{name} must be 1 or 0, got {other:?}")),
    }
}

fn parse_u64(name: &str, value: &str) -> Result<u64, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{name} must be a whole number, got {value:?}"))
}

fn parse_limit(value: &str) -> Result<u64, String> {
    match parse_u64("BACKFILL_LIMIT", value)? {
        0 => Err("BACKFILL_LIMIT must be at least 1".to_string()),
        limit => Ok(limit),
    }
}

/// Parses `YYYY-MM-DD` as midnight UTC.
///
/// # Errors
///
/// Returns a message naming `BACKFILL_SINCE` when the value is not a calendar date.
pub fn parse_since(value: &str) -> Result<DateTime<Utc>, String> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|midnight| midnight.and_utc())
        .ok_or_else(|| format!("BACKFILL_SINCE must be a date like 2026-09-01, got {value:?}"))
}

/// The report file name for a run starting now, stamped in UTC.
pub fn report_file_name() -> String {
    format!(
        "backfill-report-{}.csv",
        Utc::now().format("%Y%m%dT%H%M%SZ")
    )
}

/// What happened to one transcription; `as_str` is the CSV `outcome` value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outcome {
    WouldAttribute,
    Attributed,
    AlreadyDone,
    NobodyIdentified,
    SegmentsDiffer,
    RecallMissing,
    NotGoogleMeet,
    Error,
}

impl Outcome {
    /// Every outcome, in report order.
    pub const ALL: [Outcome; 8] = [
        Outcome::WouldAttribute,
        Outcome::Attributed,
        Outcome::AlreadyDone,
        Outcome::NobodyIdentified,
        Outcome::SegmentsDiffer,
        Outcome::RecallMissing,
        Outcome::NotGoogleMeet,
        Outcome::Error,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::WouldAttribute => "would_attribute",
            Outcome::Attributed => "attributed",
            Outcome::AlreadyDone => "already_done",
            Outcome::NobodyIdentified => "nobody_identified",
            Outcome::SegmentsDiffer => "segments_differ",
            Outcome::RecallMissing => "recall_missing",
            Outcome::NotGoogleMeet => "not_google_meet",
            Outcome::Error => "error",
        }
    }
}

/// One transcription's line in the report: ids, counts, and codes only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub transcription_id: Id,
    pub coaching_session_id: Id,
    pub outcome: Outcome,
    /// Speakers Recall reports; unknown when Recall was not reached.
    pub speakers: Option<usize>,
    pub coach: bool,
    pub coachee: bool,
    pub detail: Option<String>,
}

impl Row {
    fn new(transcription: &transcription::Model) -> Self {
        Self {
            transcription_id: transcription.id,
            coaching_session_id: transcription.coaching_session_id,
            outcome: Outcome::Error,
            speakers: None,
            coach: false,
            coachee: false,
            detail: None,
        }
    }

    fn finish(self, outcome: Outcome, detail: Option<&str>) -> Self {
        Self {
            outcome,
            detail: detail.map(str::to_string),
            ..self
        }
    }

    /// The row as a CSV line matching `CSV_HEADER`.
    pub fn csv_line(&self) -> String {
        let yes_no = |b: bool| if b { "yes" } else { "no" };
        format!(
            "{},{},{},{},{},{},{}",
            self.transcription_id,
            self.coaching_session_id,
            self.outcome.as_str(),
            self.speakers.map(|n| n.to_string()).unwrap_or_default(),
            yes_no(self.coach),
            yes_no(self.coachee),
            self.detail.as_deref().unwrap_or_default().replace(',', ";"),
        )
    }
}

/// Counts per outcome plus how many coaches and coachees were identified.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub outcomes: BTreeMap<Outcome, usize>,
    pub coach: usize,
    pub coachee: usize,
}

impl Summary {
    pub fn from_rows(rows: &[Row]) -> Self {
        Self {
            outcomes: rows.iter().fold(BTreeMap::new(), |mut counts, row| {
                *counts.entry(row.outcome).or_default() += 1;
                counts
            }),
            coach: rows.iter().filter(|r| r.coach).count(),
            coachee: rows.iter().filter(|r| r.coachee).count(),
        }
    }
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total: usize = self.outcomes.values().sum();
        writeln!(f, "transcriptions: {total}")?;
        for outcome in Outcome::ALL {
            let count = self.outcomes.get(&outcome).copied().unwrap_or_default();
            writeln!(f, "  {}: {count}", outcome.as_str())?;
        }
        writeln!(f, "coach identified: {}", self.coach)?;
        write!(f, "coachee identified: {}", self.coachee)
    }
}

/// The outside services a run reads from.
pub struct Providers<'a> {
    pub transcripts: &'a dyn transcription_trait::Provider,
    pub bot: &'a dyn recording_bot::Provider,
}

/// Opens a single-connection database; a dry run's session is verified read-only before use.
///
/// # Errors
///
/// Fails when the connection cannot be opened or a dry run cannot be made read-only.
pub async fn connect(config: &Config, apply: bool) -> Result<DatabaseConnection, Error> {
    let mut options = ConnectOptions::new(config.database_url());
    options
        .max_connections(1)
        .min_connections(1)
        .connect_timeout(Duration::from_secs(config.db_connect_timeout_secs))
        .acquire_timeout(Duration::from_secs(config.db_acquire_timeout_secs))
        .sqlx_logging(false)
        .set_schema_search_path("refactor_platform");
    if !apply {
        // Every physical connection starts read-only, even one reopened by the pool.
        options.map_sqlx_postgres_opts(|pg| pg.options([("default_transaction_read_only", "on")]));
    }

    let db = Database::connect(options)
        .await
        .map_err(EntityApiError::from)?;
    if !apply {
        make_read_only(&db).await?;
    }
    Ok(db)
}

/// Sets the session read-only and confirms Postgres reports it so.
async fn make_read_only(db: &DatabaseConnection) -> Result<(), Error> {
    db.execute_unprepared("SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY")
        .await
        .map_err(EntityApiError::from)?;
    let read_only: Option<String> = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SHOW transaction_read_only",
        ))
        .await
        .map_err(EntityApiError::from)?
        .map(|row| row.try_get("", "transaction_read_only"))
        .transpose()
        .map_err(EntityApiError::from)?;

    match read_only.as_deref() {
        Some("on") => Ok(()),
        _ => Err(other("the dry-run session is not read-only")),
    }
}

/// Whether `transcript_participants` exists; always true off Postgres.
///
/// # Errors
///
/// Fails when the catalog cannot be queried.
pub async fn participants_table_exists(db: &impl ConnectionTrait) -> Result<bool, Error> {
    if db.get_database_backend() != DatabaseBackend::Postgres {
        return Ok(true);
    }
    let present: Option<bool> = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT to_regclass('refactor_platform.transcript_participants') IS NOT NULL AS present",
        ))
        .await
        .map_err(EntityApiError::from)?
        .map(|row| row.try_get("", "present"))
        .transpose()
        .map_err(EntityApiError::from)?;
    Ok(present.unwrap_or(false))
}

/// The run's transcriptions: candidates, or every completed one before the migration has run.
///
/// # Errors
///
/// Refuses to apply when `transcript_participants` does not exist yet.
pub async fn candidates(
    db: &impl ConnectionTrait,
    options: &Options,
) -> Result<Vec<transcription::Model>, Error> {
    if participants_table_exists(db).await? {
        return find_candidates(db, options.since, options.limit).await;
    }
    if options.apply {
        return Err(other(
            "transcript_participants does not exist; run the migration before applying",
        ));
    }
    warn!("transcript_participants does not exist; treating every completed transcription as a candidate");
    Ok(completed(options.since, options.limit)
        .all(db)
        .await
        .map_err(EntityApiError::from)?)
}

/// Completed transcriptions with no participant rows, oldest first, created at or after `since`,
/// at most `limit`.
///
/// # Errors
///
/// Fails when the query fails.
pub async fn find_candidates(
    db: &impl ConnectionTrait,
    since: Option<DateTime<Utc>>,
    limit: Option<u64>,
) -> Result<Vec<transcription::Model>, Error> {
    let has_participants = Query::select()
        .expr(Expr::val(1))
        .from(Participants.table_ref())
        .and_where(
            Expr::col((Participants, ParticipantColumn::TranscriptionId))
                .equals((transcription::Entity, transcription::Column::Id)),
        )
        .to_owned();

    Ok(completed(since, limit)
        .filter(Expr::not_exists(has_participants))
        .all(db)
        .await
        .map_err(EntityApiError::from)?)
}

fn completed(since: Option<DateTime<Utc>>, limit: Option<u64>) -> Select<transcription::Entity> {
    transcription::Entity::find()
        .filter(transcription::Column::Status.eq(TranscriptionStatus::Completed))
        .apply_if(since, |query, since| {
            query.filter(transcription::Column::CreatedAt.gte(since))
        })
        .order_by_asc(transcription::Column::CreatedAt)
        .order_by_asc(transcription::Column::Id)
        .limit(limit)
}

/// A transcription's segments ordered by `(start_ms, id)`, selecting only id, start_ms, text.
///
/// # Errors
///
/// Fails when the query fails.
pub async fn stored_segments(
    db: &impl ConnectionTrait,
    transcription_id: Id,
) -> Result<Vec<StoredSegment>, Error> {
    let rows: Vec<(Id, i32, String)> = Segments::find()
        .select_only()
        .columns([
            SegmentColumn::Id,
            SegmentColumn::StartMs,
            SegmentColumn::Text,
        ])
        .filter(SegmentColumn::TranscriptionId.eq(transcription_id))
        .order_by_asc(SegmentColumn::StartMs)
        .order_by_asc(SegmentColumn::Id)
        .into_tuple()
        .all(db)
        .await
        .map_err(EntityApiError::from)?;

    Ok(rows
        .into_iter()
        .map(|(id, start_ms, text)| StoredSegment { id, start_ms, text })
        .collect())
}

/// Ok when `rebuilt` reproduces `stored` as the same multiset of (start ms, text), so segments
/// sharing a start may appear in either order. Speaker labels are not compared. The Err detail names
/// only counts or a start (never text) and contains no commas.
pub fn segments_match(
    stored: &[StoredSegment],
    rebuilt: &[transcription_types::Segment],
) -> Result<(), String> {
    if stored.len() != rebuilt.len() {
        return Err(format!(
            "segment count stored {} rebuilt {}",
            stored.len(),
            rebuilt.len()
        ));
    }
    let stored_keys = sorted_keys(
        stored
            .iter()
            .map(|s| (i64::from(s.start_ms), s.text.as_str())),
    );
    let rebuilt_keys = sorted_keys(rebuilt.iter().map(|r| (r.start_ms, r.text.as_str())));
    stored_keys
        .iter()
        .zip(&rebuilt_keys)
        .find(|(s, r)| s != r)
        .map_or(Ok(()), |(s, r)| {
            Err(format!(
                "content differs at start_ms {}",
                Ord::min(s.0, r.0)
            ))
        })
}

/// The (start ms, text) keys in sorted order, for comparing as multisets.
fn sorted_keys<'a>(keys: impl Iterator<Item = (i64, &'a str)>) -> Vec<(i64, &'a str)> {
    let mut keys: Vec<_> = keys.collect();
    keys.sort_unstable();
    keys
}

/// Pairs each stored segment with the participant of the rebuilt segment carrying the same start
/// and text, in stored order. Refuses when the content differs, or when segments sharing a start
/// and text came from different participants (who said which cannot be known).
pub(crate) fn pair_segments(
    stored: &[StoredSegment],
    rebuilt: &[transcription_types::Segment],
) -> Result<Vec<(Id, Option<String>)>, String> {
    segments_match(stored, rebuilt)?;

    let speakers_of: HashMap<(i64, &str), Vec<Option<&str>>> =
        rebuilt.iter().fold(HashMap::new(), |mut groups, r| {
            groups
                .entry((r.start_ms, r.text.as_str()))
                .or_default()
                .push(r.participant_id.as_deref());
            groups
        });

    stored
        .iter()
        .map(|segment| {
            let start = i64::from(segment.start_ms);
            let speakers = speakers_of
                .get(&(start, segment.text.as_str()))
                .ok_or_else(|| format!("content differs at start_ms {start}"))?;
            match speakers.split_first() {
                Some((first, rest)) if rest.iter().all(|p| p == first) => {
                    Ok((segment.id, first.map(str::to_string)))
                }
                _ => Err(format!("ambiguous speakers at start_ms {start}")),
            }
        })
        .collect()
}

/// In one transaction: if the transcription already has participant rows, `AlreadyDone` and no
/// writes; otherwise insert one participant row per `result.participants` (attribution from
/// `attributions` by provider id), then set `participant_id` on the EXISTING stored segments,
/// each paired with the rebuilt segment of the same start and text by `pair_segments` (unknown
/// provider ids stay NULL).
///
/// Refuses segments that do not match or cannot be paired. Never inserts, deletes, or rewrites a
/// segment's other columns.
///
/// # Errors
///
/// Fails when the segments cannot be paired or any statement fails; nothing is then written.
pub(crate) async fn link_participants(
    db: &DatabaseConnection,
    transcription_id: Id,
    result: &transcription_types::Transcription,
    attributions: &[Attribution],
    stored: &[StoredSegment],
) -> Result<Linked, Error> {
    let pairs = pair_segments(stored, &result.segments)
        .map_err(|detail| other(&format!("refusing to link: {detail}")))?;

    let txn = db.begin().await.map_err(EntityApiError::from)?;

    // Checked inside the transaction so a concurrent writer is never duplicated.
    if !participant_api::find_by_transcription(&txn, transcription_id)
        .await?
        .is_empty()
    {
        return Ok(Linked::AlreadyDone);
    }

    let participants = participant_api::create_batch(
        &txn,
        participant_rows(transcription_id, result, attributions),
    )
    .await?;
    let row_of: HashMap<&str, Id> = participants
        .iter()
        .map(|p| (p.provider_participant_id.as_str(), p.id))
        .collect();
    let segments_of: BTreeMap<Id, Vec<Id>> = pairs
        .iter()
        .filter_map(|(segment, provider_id)| {
            provider_id
                .as_deref()
                .and_then(|provider_id| row_of.get(provider_id))
                .map(|&participant| (participant, *segment))
        })
        .fold(BTreeMap::new(), |mut groups, (participant, segment)| {
            groups
                .entry(participant)
                .or_insert_with(Vec::new)
                .push(segment);
            groups
        });

    for (participant_id, segment_ids) in segments_of {
        Segments::update_many()
            .col_expr(SegmentColumn::ParticipantId, Expr::value(participant_id))
            .filter(SegmentColumn::TranscriptionId.eq(transcription_id))
            .filter(SegmentColumn::ParticipantId.is_null())
            .filter(SegmentColumn::Id.is_in(segment_ids))
            .exec(&txn)
            .await
            .map_err(EntityApiError::from)?;
    }

    txn.commit().await.map_err(EntityApiError::from)?;
    Ok(Linked::Done)
}

/// Classifies one transcription and, when applying, links its speakers.
///
/// Never fails: every problem becomes the row's outcome so the run can continue.
pub async fn process(
    db: &DatabaseConnection,
    providers: &Providers<'_>,
    config: &Config,
    transcription: &transcription::Model,
    options: &Options,
) -> Row {
    let row = Row::new(transcription);

    let session =
        match coaching_session_api::find_by_id(db, transcription.coaching_session_id).await {
            Ok(session) => session,
            Err(e) => return failed(row, "loading the session failed", &e.into()),
        };
    if session_meet_code(&session).is_none() {
        return row.finish(Outcome::NotGoogleMeet, Some("not a google meet session"));
    }

    let result = match providers
        .transcripts
        .get_transcription(&transcription.external_id)
        .await
    {
        Ok(result) if result.participants.is_empty() && result.segments.is_empty() => {
            return row.finish(Outcome::RecallMissing, Some("no transcript download"));
        }
        Ok(result) => result,
        Err(e) => return row.finish(Outcome::RecallMissing, Some(recall_error_code(&e))),
    };
    let row = Row {
        speakers: Some(result.participants.len()),
        ..row
    };

    let stored = match stored_segments(db, transcription.id).await {
        Ok(stored) => stored,
        Err(e) => return failed(row, "reading the stored segments failed", &e),
    };
    if let Err(detail) = pair_segments(&stored, &result.segments) {
        return row.finish(Outcome::SegmentsDiffer, Some(detail.as_str()));
    }

    let attributions = attribute_speakers_with(
        db,
        Some(providers.bot),
        config,
        transcription,
        &result.participants,
        options.google,
    )
    .await;
    // Left unlinked so a later run can retry it.
    if attributions.iter().all(|a| a.user_id.is_none()) {
        return row.finish(Outcome::NobodyIdentified, Some("no speaker identified"));
    }
    let identified = |source| attributions.iter().any(|a| a.source == Some(source));
    let row = Row {
        coach: identified(MatchSource::Account),
        coachee: identified(MatchSource::Elimination),
        ..row
    };

    if !options.apply {
        return row.finish(Outcome::WouldAttribute, None);
    }
    match link_participants(db, transcription.id, &result, &attributions, &stored).await {
        Ok(Linked::Done) => row.finish(Outcome::Attributed, None),
        Ok(Linked::AlreadyDone) => {
            row.finish(Outcome::AlreadyDone, Some("participants already stored"))
        }
        Err(e) => failed(row, "linking failed", &e),
    }
}

/// An `error` row; the log names only the transcription and the error kind.
fn failed(row: Row, detail: &str, e: &Error) -> Row {
    warn!(
        "Backfill for transcription {}: {detail}: {:?}",
        row.transcription_id, e.error_kind
    );
    row.finish(Outcome::Error, Some(detail))
}

/// A short code for a Recall failure; provider messages are never repeated.
fn recall_error_code(e: &meeting_ai::Error) -> &'static str {
    match e {
        meeting_ai::Error::Authentication(_) => "recall authentication failed",
        meeting_ai::Error::Network(_) => "recall network error",
        meeting_ai::Error::Configuration(_) => "recall configuration error",
        meeting_ai::Error::Provider(_) => "recall provider error",
        meeting_ai::Error::Timeout(_) => "recall timeout",
        meeting_ai::Error::NotFound(_) => "recall not found",
        meeting_ai::Error::RateLimited { .. } => "recall rate limited",
        meeting_ai::Error::Serialization(_) | meeting_ai::Error::Deserialization(_) => {
            "recall response unreadable"
        }
        meeting_ai::Error::Other(_) => "recall error",
    }
}

fn other(message: &str) -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(message.to_string())),
    }
}

#[cfg(test)]
#[path = "transcript_backfill_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "transcript_backfill_sqlite_tests.rs"]
mod sqlite_tests;

#[cfg(test)]
#[path = "transcript_backfill_run_tests.rs"]
mod run_tests;

#[cfg(test)]
#[path = "transcript_backfill_ties_tests.rs"]
mod ties_tests;
