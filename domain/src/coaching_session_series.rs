//! Domain orchestration for `coaching_session_series` — the entity that owns
//! a recurrence rule and groups the materialized `coaching_sessions` rows
//! created from it.
//!
//! This module owns the JSONB rule serialization: callers interact with the
//! typed [`SeriesRule`] struct, never with `serde_json::Value` directly.

use crate::coaching_session;
use crate::coaching_session_image;
use crate::coaching_sessions;
use crate::duration::Duration;
use crate::emails;
use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::gateway::ical;
use crate::gateway::tiptap::TiptapDocument;
use crate::Id;
use chrono::NaiveDateTime;
use chrono_tz::Tz;
use entity_api::coaching_session_series;
use futures::stream::{self, StreamExt, TryStreamExt};
use log::{debug, warn};
use sea_orm::{DatabaseConnection, TransactionTrait};
use serde::{Deserialize, Serialize};
use service::config::Config;

pub use coaching_session::{Frequency, Recurrence, RecurrenceError};
pub use entity_api::coaching_session_series::find_by_id;
pub use entity_api::coaching_session_series::Model;

/// Typed shape of the JSONB `rule` column on `coaching_session_series`.
/// Stored at create time and re-read at reschedule time, which is why
/// `duration_minutes` is captured here — a coach who updates their default
/// duration between create and reschedule shouldn't see the existing series
/// flip durations under them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeriesRule {
    pub start_at: NaiveDateTime,
    pub recurrence: Recurrence,
    pub duration_minutes: i16,
    /// IANA zone the series was published in: its invites are written in it and its
    /// slots expanded in it, so a coach changing zone later cannot split the two.
    #[serde(default = "default_timezone")]
    pub timezone: String,
}

fn default_timezone() -> String {
    chrono_tz::UTC.name().to_string()
}

impl SeriesRule {
    /// The zone to expand and publish in. An unknown name falls back to UTC, as an
    /// invite for it would.
    pub fn zone(&self) -> Tz {
        self.timezone
            .parse()
            .map_or(chrono_tz::UTC, ical::emitted_zone)
    }

    /// The original start of every occurrence, in the form `ical_recurrence_id` stores.
    pub(crate) fn slots(&self) -> Result<Vec<NaiveDateTime>, Error> {
        coaching_session::series_slots(self.start_at, &self.recurrence, self.zone())
    }
}

/// Create a series and materialize its sessions in a single transaction.
///
/// 1. Authorizes the caller: `acting_user_id` must be the coach on the target
///    relationship (the relationship id arrives in the request body, so this
///    coach-only gate lives here rather than in a `FromRequestParts` extractor
///    like the path/query-keyed series routes).
/// 2. Expands and validates the recurrence rule.
/// 3. Resolves the coach's effective duration via the defaulting cascade.
/// 4. Inserts the series row with the resolved rule (including the resolved
///    `duration_minutes`) serialized to JSONB.
/// 5. Bulk-inserts the materialized sessions linked back to the series via
///    `coaching_session_series_id`.
///
/// Returns `(series, sessions)`. Tiptap docs, meeting URLs, and goal links
/// are NOT created here — those run lazily on first read via
/// `coaching_session::ensure_hydrated`.
pub async fn create_with_sessions(
    db: &DatabaseConnection,
    config: &Config,
    coaching_relationship_id: Id,
    acting_user_id: Id,
    start_at: NaiveDateTime,
    recurrence: Recurrence,
    requested_duration: Option<Duration>,
) -> Result<(Model, Vec<coaching_sessions::Model>), Error> {
    let relationship =
        entity_api::coaching_relationship::find_by_id(db, coaching_relationship_id).await?;
    if relationship.coach_id != acting_user_id {
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::Forbidden,
            )),
        });
    }
    let coach_id = relationship.coach_id;
    let zone = publication_zone(db, coach_id).await?;

    let dates = coaching_session::expand_recurrence(start_at, &recurrence, zone)?;
    let resolved_duration =
        entity_api::coaching_session::resolve_duration(db, coach_id, requested_duration).await?;

    let rule = SeriesRule {
        start_at,
        recurrence,
        duration_minutes: resolved_duration.minutes(),
        timezone: zone.name().to_string(),
    };
    let rule_json = serde_json::to_value(&rule)?;

    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;

    let series_input = Model {
        id: Id::nil(),
        coaching_relationship_id,
        rule: rule_json,
        ical_sequence: 0,
        created_by_user_id: acting_user_id,
        created_at: chrono::Utc::now().into(),
        updated_at: chrono::Utc::now().into(),
    };
    let series = coaching_session_series::create(&txn, series_input).await?;

    let sessions = coaching_session::bulk_create_recurring(
        &txn,
        coaching_relationship_id,
        coach_id,
        series.id,
        dates,
        Some(resolved_duration),
    )
    .await?;

    txn.commit().await.map_err(entity_api::error::Error::from)?;

    // Best-effort, after commit: a failed invite must not undo a created series.
    emails::notify_recurring_sessions_scheduled(db, config, &series, &sessions).await;

    Ok((series, sessions))
}

/// The zone a series published now by this coach is written in.
async fn publication_zone(db: &DatabaseConnection, coach_id: Id) -> Result<Tz, Error> {
    let coach = entity_api::user::find_by_id_without_roles(db, coach_id).await?;
    Ok(emails::publication_zone(&coach))
}

/// List every series owned by the given coaching relationship.
pub async fn find_by_relationship(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(coaching_session_series::find_by_relationship(db, coaching_relationship_id).await?)
}

/// Outcome of a reschedule request.
#[derive(Debug)]
pub struct Rescheduled {
    pub series: Model,
    pub sessions: Vec<coaching_sessions::Model>,
    /// False when the requested rule already matched the stored one, so nothing was
    /// re-materialized and no invites are owed.
    pub changed: bool,
}

/// Replace the rule on an existing series and re-materialize its future
/// sessions. Future sessions are deleted unconditionally: goal links, topics,
/// recordings, and transcriptions cascade via the FK, but notes, actions and
/// agreements do not, so a future session holding any of them fails the delete.
/// Tiptap documents are cleaned up best-effort after the DB transaction commits.
///
/// A request whose rule matches the stored one short-circuits: re-materializing
/// would destroy live session rows and their notes to recreate an identical
/// schedule, and would re-send invites under a burned SEQUENCE.
pub async fn reschedule(
    db: &DatabaseConnection,
    config: &Config,
    series_id: Id,
    coach_id: Id,
    new_start_at: NaiveDateTime,
    new_recurrence: Recurrence,
    new_requested_duration: Option<Duration>,
) -> Result<Rescheduled, Error> {
    // Validate new inputs before any DB write. A reschedule only ever touches
    // future sessions (past sessions are left untouched), so a past
    // `new_start_at` would re-materialize the past on top of surviving history
    // — reject it up front rather than corrupt the timeline.
    let now_naive = chrono::Utc::now().naive_utc();
    if new_start_at < now_naive {
        return Err(RecurrenceError::StartAtInPast.into());
    }

    // A reschedule re-publishes the series, so it adopts the coach's zone as it is now.
    let zone = publication_zone(db, coach_id).await?;
    let new_dates = coaching_session::expand_recurrence(new_start_at, &new_recurrence, zone)?;

    // Resolve the duration for the re-materialized sessions. A reschedule moves
    // the meetings; it must not silently restretch them. So when the caller
    // omits a duration, reuse the value persisted on the existing series rule
    // rather than re-deriving the coach's *current* default (which may have
    // changed since the series was created). An explicit request still wins.
    let existing = coaching_session_series::find_by_id(db, series_id).await?;
    // An unreadable stored rule is treated as "differs", which re-materializes rather
    // than wrongly reporting no change.
    let existing_rule: Option<SeriesRule> = serde_json::from_value(existing.rule.clone()).ok();
    let resolved_duration = match (new_requested_duration, &existing_rule) {
        (Some(duration), _) => duration,
        (None, Some(rule)) => Duration::from_minutes_unchecked(rule.duration_minutes),
        // Only reachable when the stored rule is unreadable; re-parse so the caller
        // sees the actual deserialization error rather than an invented one.
        (None, None) => serde_json::from_value::<SeriesRule>(existing.rule.clone())
            .map(|rule| Duration::from_minutes_unchecked(rule.duration_minutes))?,
    };

    let new_rule = SeriesRule {
        start_at: new_start_at,
        recurrence: new_recurrence,
        duration_minutes: resolved_duration.minutes(),
        timezone: zone.name().to_string(),
    };

    let future_sessions =
        entity_api::coaching_session::find_future_sessions_by_series_id(db, series_id, now_naive)
            .await?;

    // Compared as parsed rules, not as JSON: field order, an omitted-vs-null optional, or
    // a field that gained a serde default would all read as "changed" on a blob compare.
    if existing_rule.as_ref() == Some(&new_rule) {
        debug!("Reschedule of series {series_id} matches the stored rule; nothing to do");
        return Ok(Rescheduled {
            series: existing,
            sessions: future_sessions,
            changed: false,
        });
    }

    let new_rule = serde_json::to_value(&new_rule)?;

    // Snapshot Tiptap docs to clean up *after* the DB txn commits.
    let doc_names_to_cleanup: Vec<String> = future_sessions
        .iter()
        .filter_map(|s| s.collab_document_name.clone())
        .collect();
    let future_ids: Vec<Id> = future_sessions.iter().map(|s| s.id).collect();

    // Read before the delete: the image rows' FK cascades, and `storage_key` is the only
    // pointer to the bytes.
    let image_keys = coaching_session_image::storage_keys_for_sessions(db, &future_ids).await?;

    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;

    for id in &future_ids {
        entity_api::coaching_session::acquire_advisory_lock(&txn, *id).await?;
    }

    // Re-read under the locks. The snapshot above was taken before them, so an individual
    // reschedule committing in between would have bumped an `ical_sequence` it does not
    // show. Cancelling a standalone event off that stale number reuses a `SEQUENCE` the
    // client has already applied, and it drops the cancellation as a duplicate without
    // erroring anywhere.
    let replaced_sessions =
        entity_api::coaching_session::find_future_sessions_by_series_id(&txn, series_id, now_naive)
            .await?;

    entity_api::coaching_session::bulk_delete_by_ids(&txn, &future_ids).await?;

    let updated_series = coaching_session_series::update_rule(&txn, series_id, new_rule).await?;

    let new_sessions = coaching_session::bulk_create_recurring(
        &txn,
        updated_series.coaching_relationship_id,
        coach_id,
        updated_series.id,
        new_dates,
        Some(resolved_duration),
    )
    .await?;

    txn.commit().await.map_err(entity_api::error::Error::from)?;

    cleanup_orphaned_docs(config, series_id, "reschedule", &doc_names_to_cleanup).await;
    coaching_session_image::destroy_objects(config, &image_keys).await;

    // Best-effort, after commit. Fired here rather than from the caller because the
    // previous rule is only in scope inside this function.
    emails::notify_recurring_sessions_rescheduled(
        db,
        config,
        &updated_series,
        emails::PreviousSeries(&existing),
        &new_sessions,
        emails::ReplacedSessions(&replaced_sessions),
    )
    .await;

    Ok(Rescheduled {
        series: updated_series,
        sessions: new_sessions,
        changed: true,
    })
}

/// A "this and following" reschedule: one session of a series and every session after it.
#[derive(Debug)]
pub struct FollowingReschedule {
    /// The first session to move.
    pub coaching_session_id: Id,
    /// The new start of that session.
    pub start_at: NaiveDateTime,
    /// `None` keeps the series' current duration.
    pub duration: Option<Duration>,
    /// `None` keeps the series' cadence, carried on from `start_at`.
    pub recurrence: Option<Recurrence>,
}

/// How a "this and following" reschedule lands, worked out before anything is written.
#[derive(Debug, PartialEq)]
struct FollowingPlan {
    /// The rule the moved sessions are published under.
    rule: SeriesRule,
    /// The old series' rule cut short before the first moved session, when earlier
    /// occurrences stay behind on it.
    split_from: Option<SeriesRule>,
    /// Each affected session, paired with the slot it moves to.
    moves: Vec<(coaching_sessions::Model, NaiveDateTime)>,
    /// Slots past the last affected session, materialized as new sessions.
    extras: Vec<NaiveDateTime>,
    /// Affected sessions past the last slot, which are deleted.
    leftovers: Vec<coaching_sessions::Model>,
}

impl FollowingPlan {
    fn duration(&self) -> Duration {
        Duration::from_minutes_unchecked(self.rule.duration_minutes)
    }

    /// True when every affected session already sits on its slot at the new duration, and
    /// nothing is added or removed.
    fn changes_nothing(&self) -> bool {
        self.extras.is_empty()
            && self.leftovers.is_empty()
            && self.moves.iter().all(|(session, slot)| {
                session.date == *slot && session.duration_minutes == self.rule.duration_minutes
            })
    }

    /// The sessions the reschedule touches, in the order they are locked.
    fn affected_ids(&self) -> Vec<Id> {
        self.moves
            .iter()
            .map(|(session, _)| session)
            .chain(&self.leftovers)
            .map(|session| session.id)
            .collect()
    }

    fn leftover_ids(&self) -> Vec<Id> {
        self.leftovers.iter().map(|session| session.id).collect()
    }
}

/// A series' sessions in the order a reschedule walks them: by date, ties by id.
async fn series_sessions_in_order(
    db: &impl sea_orm::ConnectionTrait,
    series_id: Id,
) -> Result<Vec<coaching_sessions::Model>, Error> {
    let mut sessions = entity_api::coaching_session::find_by_series_id(db, series_id).await?;
    sessions.sort_by_key(|session| (session.date, session.id));
    Ok(sessions)
}

fn invalid(message: &str) -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Validation(message.to_string()),
    }
}

/// Pair the affected sessions with the new rule's slots. Pure, so every edge of the
/// pairing is testable without a database.
///
/// The series splits only when the old rule has an occurrence before the first moved
/// session that a session still holds. Otherwise the rule is replaced in place, as a full
/// series reschedule does.
fn plan_following(
    existing: &SeriesRule,
    sessions: &[coaching_sessions::Model],
    request: &FollowingReschedule,
    zone: Tz,
    now: NaiveDateTime,
) -> Result<FollowingPlan, Error> {
    let anchor_index = sessions
        .iter()
        .position(|session| session.id == request.coaching_session_id)
        .ok_or_else(|| invalid("`coaching_session_id` is not a session of this series."))?;
    let (earlier, affected) = sessions.split_at(anchor_index);
    let anchor = &affected[0];
    if anchor.date < now {
        return Err(invalid(
            "`coaching_session_id` has already happened; only upcoming sessions can be rescheduled.",
        ));
    }
    if request.start_at < now {
        return Err(RecurrenceError::StartAtInPast.into());
    }

    let recurrence = request
        .recurrence
        .clone()
        .unwrap_or_else(|| existing.recurrence.continued(affected.len()));
    let slots = coaching_session::publishable_slots(request.start_at, &recurrence, zone)?;
    let rule = SeriesRule {
        start_at: request.start_at,
        recurrence,
        duration_minutes: request
            .duration
            .map_or(existing.duration_minutes, |duration| duration.minutes()),
        timezone: zone.name().to_string(),
    };

    let anchor_slot = anchor.ical_recurrence_id.unwrap_or(anchor.date);
    let has_earlier_occurrence = existing
        .slots()?
        .first()
        .is_some_and(|first| *first < anchor_slot);
    let split_from = (!earlier.is_empty() && has_earlier_occurrence).then(|| SeriesRule {
        recurrence: existing.recurrence.ending_before(anchor_slot),
        ..existing.clone()
    });

    Ok(FollowingPlan {
        moves: affected
            .iter()
            .cloned()
            .zip(slots.iter().copied())
            .collect(),
        extras: slots.iter().skip(affected.len()).copied().collect(),
        leftovers: affected.iter().skip(slots.len()).cloned().collect(),
        rule,
        split_from,
    })
}

/// Reschedule one session of a series and every session after it, in place.
///
/// The k-th affected session moves to the k-th slot of the new rule and keeps its id and
/// everything hung off it. Extra slots become new sessions. Leftover sessions are deleted,
/// unless one holds content, which refuses the whole request with a 409 instead. When
/// earlier occurrences stay behind, the series splits: the old one's rule ends before the
/// moved session and a new series holds the moved ones, as calendars model "this and
/// following". A request that changes nothing writes nothing and sends no email.
pub async fn reschedule_following(
    db: &DatabaseConnection,
    config: &Config,
    series_id: Id,
    coach_id: Id,
    request: FollowingReschedule,
) -> Result<Rescheduled, Error> {
    let existing = coaching_session_series::find_by_id(db, series_id).await?;
    let existing_rule: SeriesRule = serde_json::from_value(existing.rule.clone())?;
    let zone = publication_zone(db, coach_id).await?;
    let now = chrono::Utc::now().naive_utc();

    // Planned once to reject a bad request before taking locks, then again under them.
    let snapshot = series_sessions_in_order(db, series_id).await?;
    let draft = plan_following(&existing_rule, &snapshot, &request, zone, now)?;

    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;
    for id in draft.affected_ids() {
        entity_api::coaching_session::acquire_advisory_lock(&txn, id).await?;
    }
    let sessions = series_sessions_in_order(&txn, series_id).await?;
    let plan = plan_following(&existing_rule, &sessions, &request, zone, now)?;

    if plan.changes_nothing() {
        debug!("Following reschedule of series {series_id} matches its sessions; nothing to do");
        return Ok(Rescheduled {
            series: existing,
            sessions: plan.moves.into_iter().map(|(session, _)| session).collect(),
            changed: false,
        });
    }

    let leftover_ids = plan.leftover_ids();
    let with_content =
        entity_api::coaching_session::find_ids_with_content(&txn, &leftover_ids).await?;
    if !with_content.is_empty() {
        let blocking: Vec<Id> = leftover_ids
            .iter()
            .copied()
            .filter(|id| with_content.contains(id))
            .collect();
        return Err(coaching_session::conflict(
            "The new schedule leaves out sessions that have notes, actions, agreements or topics.",
            serde_json::json!({
                "reason": "sessions_have_content",
                "coaching_session_ids": blocking,
            }),
        ));
    }

    // Read before the delete: the image rows' FK cascades, and `storage_key` is the only
    // pointer to the bytes.
    let image_keys = coaching_session_image::storage_keys_for_sessions(&txn, &leftover_ids).await?;

    let (target, split_from) = match &plan.split_from {
        Some(shortened) => {
            let split_from = coaching_session_series::update_rule(
                &txn,
                series_id,
                serde_json::to_value(shortened)?,
            )
            .await?;
            let target = coaching_session_series::create(
                &txn,
                Model {
                    id: Id::nil(),
                    coaching_relationship_id: existing.coaching_relationship_id,
                    rule: serde_json::to_value(&plan.rule)?,
                    ical_sequence: 0,
                    created_by_user_id: coach_id,
                    created_at: chrono::Utc::now().into(),
                    updated_at: chrono::Utc::now().into(),
                },
            )
            .await?;
            (target, Some(split_from))
        }
        None => (
            coaching_session_series::update_rule(
                &txn,
                series_id,
                serde_json::to_value(&plan.rule)?,
            )
            .await?,
            None,
        ),
    };

    let duration = plan.duration();
    let moved: Vec<coaching_sessions::Model> = stream::iter(&plan.moves)
        .then(|(session, slot)| {
            entity_api::coaching_session::move_to_slot(&txn, session.id, target.id, *slot, duration)
        })
        .try_collect()
        .await?;
    let created = coaching_session::bulk_create_recurring(
        &txn,
        existing.coaching_relationship_id,
        coach_id,
        target.id,
        plan.extras.clone(),
        Some(duration),
    )
    .await?;
    entity_api::coaching_session::bulk_delete_by_ids(&txn, &leftover_ids).await?;

    txn.commit().await.map_err(entity_api::error::Error::from)?;

    let doc_names: Vec<String> = plan
        .leftovers
        .iter()
        .filter_map(|session| session.collab_document_name.clone())
        .collect();
    cleanup_orphaned_docs(config, series_id, "reschedule", &doc_names).await;
    coaching_session_image::destroy_objects(config, &image_keys).await;

    let sessions: Vec<coaching_sessions::Model> = moved.iter().chain(&created).cloned().collect();
    let told = emails::notify_recurring_sessions_rescheduled_following(
        db,
        config,
        &emails::FollowingRescheduled {
            series: &target,
            sessions: &sessions,
            previous_series: emails::PreviousSeries(&existing),
            previous_start: plan
                .moves
                .first()
                .map_or(request.start_at, |(session, _)| session.date),
            replaced: emails::ReplacedSessions(&plan.leftovers),
            split_from: split_from.as_ref(),
        },
    )
    .await;
    if told {
        mark_noticed(db, &moved).await;
    }

    Ok(Rescheduled {
        series: target,
        sessions,
        changed: true,
    })
}

/// Record that the participants were told about each moved session's new start, so the
/// reminder sweep measures notice from the email they just got. Best-effort per session.
async fn mark_noticed(db: &DatabaseConnection, sessions: &[coaching_sessions::Model]) {
    for session in sessions {
        if let Err(e) = entity_api::coaching_session::mark_notice_given(db, session.id).await {
            warn!(
                "Told the participants session {} moved but could not record it, so they \
                 may be reminded about a time they already have: {e:?}",
                session.id
            );
        }
    }
}

/// Delete the series row and its future sessions. Past sessions survive as
/// orphan one-offs: the FK's `ON DELETE SET NULL` clears
/// `coaching_session_series_id` for every row that the explicit future
/// delete didn't touch, so their notes / meeting URLs / collab docs stay
/// intact.
///
pub async fn delete_with_future_sessions(
    db: &DatabaseConnection,
    config: &Config,
    series_id: Id,
) -> Result<(), Error> {
    let now_naive = chrono::Utc::now().naive_utc();
    let future_sessions =
        entity_api::coaching_session::find_future_sessions_by_series_id(db, series_id, now_naive)
            .await?;

    let doc_names_to_cleanup: Vec<String> = future_sessions
        .iter()
        .filter_map(|s| s.collab_document_name.clone())
        .collect();
    let future_ids: Vec<Id> = future_sessions.iter().map(|s| s.id).collect();

    // Read before the delete: the image rows' FK cascades, and `storage_key` is the only
    // pointer to the bytes.
    let image_keys = coaching_session_image::storage_keys_for_sessions(db, &future_ids).await?;

    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;

    for id in &future_ids {
        entity_api::coaching_session::acquire_advisory_lock(&txn, *id).await?;
    }

    // Re-read under the locks, for the same reason the series SEQUENCE is bumped inside
    // this transaction rather than in memory beforehand: the snapshot above predates the
    // locks, and a per-session cancellation built off a stale `ical_sequence` reuses a
    // number the client already applied, which it drops as a duplicate.
    let cancelled_sessions =
        entity_api::coaching_session::find_future_sessions_by_series_id(&txn, series_id, now_naive)
            .await?;

    entity_api::coaching_session::bulk_delete_by_ids(&txn, &future_ids).await?;

    // Bump the SEQUENCE inside the delete transaction so the cancellation outranks any
    // reschedule that commits alongside it. Reading the series before the transaction and
    // adding one in memory would let a concurrent reschedule claim the same number, and a
    // calendar client that already applied that reschedule drops the equal-SEQUENCE
    // cancellation as a duplicate. Best-effort: deleting a nonexistent series still
    // succeeds silently, it just cannot announce a cancellation.
    let cancelled = coaching_session_series::increment_ical_sequence(&txn, series_id)
        .await
        .ok();

    coaching_session_series::delete(&txn, series_id).await?;

    txn.commit().await.map_err(entity_api::error::Error::from)?;

    cleanup_orphaned_docs(config, series_id, "delete", &doc_names_to_cleanup).await;
    coaching_session_image::destroy_objects(config, &image_keys).await;

    if let Some(series) = cancelled {
        emails::notify_recurring_sessions_cancelled(db, config, &series, &cancelled_sessions).await;
    }

    Ok(())
}

/// Resend a series' invite to both participants as it stands today, under a bumped
/// SEQUENCE so calendars replace the recurring event they hold. Refused when nothing is
/// left to attend, or when an upcoming member predates `ical_recurrence_id`: such a series
/// was never published as one, so a snapshot would not line up with what calendars hold.
pub async fn resend_invite(
    db: &DatabaseConnection,
    config: &Config,
    series_id: Id,
) -> Result<(), Error> {
    let series = coaching_session_series::find_by_id(db, series_id).await?;
    let rows = entity_api::coaching_session::find_by_series_id(db, series_id).await?;
    let now = chrono::Utc::now().naive_utc();
    let mut upcoming: Vec<coaching_sessions::Model> =
        rows.iter().filter(|s| s.date > now).cloned().collect();
    upcoming.sort_by_key(|s| s.date);

    if upcoming.is_empty() {
        return Err(coaching_session::resend_refused(
            "This series has no upcoming sessions.",
            "no_upcoming_sessions",
        ));
    }
    if upcoming.iter().any(|s| s.ical_recurrence_id.is_none()) {
        return Err(coaching_session::resend_refused(
            "This series predates resendable invites.",
            "legacy_series",
        ));
    }

    // Committed before the send: a failed send leaves a gap in SEQUENCE, which is harmless.
    // Members are raised with the series because Google drops an override whose SEQUENCE
    // trails its master.
    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;
    let bumped = coaching_session_series::increment_ical_sequence(&txn, series.id).await?;
    let rows = entity_api::coaching_session::raise_series_ical_sequence(
        &txn,
        series.id,
        bumped.ical_sequence,
    )
    .await?;
    txn.commit().await.map_err(entity_api::error::Error::from)?;

    let mut upcoming: Vec<coaching_sessions::Model> =
        rows.iter().filter(|s| s.date > now).cloned().collect();
    upcoming.sort_by_key(|s| s.date);
    let parties = emails::InviteParties::load(db, bumped.coaching_relationship_id).await?;
    let delivered = emails::send_series_invite_resend(
        db,
        config,
        &bumped,
        &parties.participants(),
        &upcoming,
        &rows,
        &parties.organization,
    )
    .await?;
    coaching_session::ensure_coachee_reached(&delivered)
}

async fn cleanup_orphaned_docs(
    config: &Config,
    series_id: Id,
    operation: &str,
    doc_names: &[String],
) {
    if doc_names.is_empty() {
        return;
    }
    match TiptapDocument::new(config).await {
        Ok(tiptap) => {
            for name in doc_names {
                if let Err(err) = tiptap.delete(name).await {
                    warn!(
                        "Tiptap cleanup failed for orphaned doc {name:?} after series \
                         {operation} {series_id}: {err}"
                    );
                }
            }
        }
        Err(err) => {
            warn!(
                "Could not construct Tiptap client to clean up {} orphaned doc(s) \
                 after series {operation} {series_id}: {err}",
                doc_names.len()
            );
        }
    }
}

#[cfg(test)]
#[path = "coaching_session_series_following_tests.rs"]
mod following_tests;

#[cfg(test)]
#[path = "coaching_session_series_sqlite_tests.rs"]
mod sqlite_tests;

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use crate::coaching_session::Frequency;
    use crate::coaching_session_images;
    use chrono::NaiveDate;
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

    fn weekly_rule_count(n: u32) -> Recurrence {
        Recurrence {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekdays: None,
            count: Some(n),
            until: None,
        }
    }

    /// The coach row the series reads its publication zone from.
    fn coach_row(timezone: &str) -> entity::users::Model {
        let now = chrono::Utc::now();
        entity::users::Model {
            id: Id::new_v4(),
            email: "coach@example.com".into(),
            first_name: "Coach".into(),
            last_name: "One".into(),
            display_name: None,
            password: None,
            github_username: None,
            github_profile_url: None,
            timezone: timezone.into(),
            default_coaching_session_duration_minutes: 60,
            roles: vec![],
            invite_status: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn start() -> NaiveDateTime {
        // Relative to now so `reschedule`'s past-guard (and date rollover)
        // can't make this flaky; a fixed calendar date eventually goes stale.
        (chrono::Utc::now() + chrono::Duration::days(7)).naive_utc()
    }

    #[tokio::test]
    async fn create_with_sessions_inserts_series_and_sessions_in_one_transaction(
    ) -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let coach_id = Id::new_v4();
        let now = chrono::Utc::now();
        let rule = weekly_rule_count(3);

        // The acting user is the coach on this relationship, so authz passes.
        let relationship = entity::coaching_relationships::Model {
            id: relationship_id,
            organization_id: Id::new_v4(),
            coach_id,
            coachee_id: Id::new_v4(),
            slug: "test".into(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let series = Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            rule: serde_json::json!({}),
            ical_sequence: 0,
            created_by_user_id: coach_id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let coach = entity::users::Model {
            id: coach_id,
            email: "coach@example.com".into(),
            first_name: "Coach".into(),
            last_name: "One".into(),
            display_name: None,
            password: None,
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".into(),
            default_coaching_session_duration_minutes: 60,
            roles: vec![],
            invite_status: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let make_session = |date: NaiveDateTime| coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: Some(series.id),
            ical_sequence: 0,
            ical_recurrence_id: None,
            collab_document_name: None,
            date,
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
            notice_given_at: chrono::Utc::now().into(),
        };

        let expected_sessions = vec![
            make_session(start()),
            make_session(start() + chrono::Duration::days(7)),
            make_session(start() + chrono::Duration::days(14)),
        ];

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // 1. authz → coaching_relationship::find_by_id
            .append_query_results(vec![vec![relationship.clone()]])
            // 2. publication zone → the coach
            .append_query_results(vec![vec![coach_row("UTC")]])
            // 3. resolve_duration → user::find_by_id with related user_roles
            .append_query_results::<(entity::users::Model, Option<entity::user_roles::Model>), _, _>(
                vec![vec![(coach, None)]],
            )
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results(vec![vec![series.clone()]])
            .append_query_results(vec![expected_sessions.clone()])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let (returned_series, returned_sessions) = create_with_sessions(
            &db,
            &test_config(),
            relationship_id,
            coach_id,
            start(),
            rule,
            None,
        )
        .await?;

        assert_eq!(returned_series.id, series.id);
        assert_eq!(returned_sessions.len(), 3);
        assert!(returned_sessions
            .iter()
            .all(|s| s.coaching_session_series_id == Some(series.id)));
        Ok(())
    }

    #[tokio::test]
    async fn create_with_sessions_rejects_invalid_recurrence() {
        let coach_id = Id::new_v4();
        let relationship_id = Id::new_v4();
        let now = chrono::Utc::now();
        // Authz must pass first, so the failure is attributable to the rule.
        let relationship = entity::coaching_relationships::Model {
            id: relationship_id,
            organization_id: Id::new_v4(),
            coach_id,
            coachee_id: Id::new_v4(),
            slug: "test".into(),
            created_at: now.into(),
            updated_at: now.into(),
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![relationship]])
            .append_query_results(vec![vec![coach_row("UTC")]])
            .into_connection();
        let bad_rule = Recurrence {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekdays: None,
            count: Some(3),
            until: Some(start() + chrono::Duration::days(21)),
        };
        let result = create_with_sessions(
            &db,
            &test_config(),
            relationship_id,
            coach_id,
            start(),
            bad_rule,
            Some(Duration::default()),
        )
        .await;
        assert!(result.is_err());
    }

    /// Happy-path reschedule over unhydrated future sessions, with the duration
    /// omitted: validates inputs, reuses the duration persisted on the existing
    /// series rule (no coach lookup), deletes the old future rows, rewrites the
    /// rule, and bulk-inserts the new schedule — all in one transaction. Tiptap
    /// is never contacted because no future session has a collab doc.
    #[tokio::test]
    async fn reschedule_replaces_future_unhydrated_sessions() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let coach_id = Id::new_v4();
        let series_id = Id::new_v4();
        let now = chrono::Utc::now();

        // The series as it currently exists: its rule carries the duration the
        // None-path reschedule must reuse instead of re-deriving a default.
        let existing_series = Model {
            id: series_id,
            coaching_relationship_id: relationship_id,
            rule: serde_json::to_value(SeriesRule {
                start_at: start(),
                recurrence: weekly_rule_count(3),
                duration_minutes: 60,
                timezone: "UTC".into(),
            })?,
            ical_sequence: 0,
            created_by_user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let make_session = |date: NaiveDateTime, doc: Option<String>| coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: Some(series_id),
            ical_sequence: 0,
            ical_recurrence_id: None,
            collab_document_name: doc,
            date,
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
            notice_given_at: chrono::Utc::now().into(),
        };

        // Two future sessions, neither hydrated, neither carrying a Tiptap doc.
        let future_sessions = vec![
            make_session(start() + chrono::Duration::days(7), None),
            make_session(start() + chrono::Duration::days(14), None),
        ];

        // The series row as returned by update_rule's UPDATE ... RETURNING.
        let updated_series = Model {
            id: series_id,
            coaching_relationship_id: relationship_id,
            rule: serde_json::json!({}),
            ical_sequence: 0,
            created_by_user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        // New schedule: 3 weekly occurrences starting at start().
        let new_sessions = vec![
            make_session(start(), None),
            make_session(start() + chrono::Duration::days(7), None),
            make_session(start() + chrono::Duration::days(14), None),
        ];

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // 0. publication zone → the coach
            .append_query_results(vec![vec![coach_row("UTC")]])
            // 1. duration omitted → find_by_id on the series to read its stored rule
            .append_query_results(vec![vec![existing_series.clone()]])
            // 2. find_future_sessions_by_series_id → 2 future rows
            .append_query_results(vec![future_sessions.clone()])
            // 2b. storage_keys_for_sessions → no images to strand
            .append_query_results(vec![Vec::<coaching_session_images::Model>::new()])
            // 3. BEGIN
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // 4. acquire_advisory_lock × 2 (one exec per session)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // 5. re-read of the future sessions, now under the locks
            .append_query_results(vec![future_sessions.clone()])
            // 6. bulk_delete_by_ids → DELETE
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 2,
            }])
            // 7. update_rule → a single UPDATE ... RETURNING
            .append_query_results(vec![vec![updated_series.clone()]])
            // 8. bulk_create_recurring → INSERT ... RETURNING
            .append_query_results(vec![new_sessions.clone()])
            // 9. COMMIT
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        let rescheduled = reschedule(
            &db,
            &test_config(),
            series_id,
            coach_id,
            start(),
            weekly_rule_count(3),
            None,
        )
        .await?;

        assert!(rescheduled.changed);
        assert_eq!(rescheduled.series.id, series_id);
        assert_eq!(rescheduled.sessions.len(), 3);
        assert!(rescheduled
            .sessions
            .iter()
            .all(|s| s.coaching_session_series_id == Some(series_id)));
        Ok(())
    }

    /// A reschedule whose rule matches the stored one must not re-materialize.
    /// Doing so would delete live session rows (taking their notes and goal links
    /// with them) only to recreate an identical schedule, and would burn a
    /// SEQUENCE re-inviting both people to a meeting that never moved.
    /// Proven by the transaction log: no BEGIN, no DELETE, no UPDATE.
    #[tokio::test]
    async fn reschedule_with_an_unchanged_rule_is_a_no_op() -> Result<(), Error> {
        let series_id = Id::new_v4();
        let now = chrono::Utc::now();
        // One `start()` value, reused: the helper is `now() + 7d`, so two calls differ.
        let start_at = start();
        let stored_rule = serde_json::to_value(SeriesRule {
            start_at,
            recurrence: weekly_rule_count(3),
            duration_minutes: 60,
            timezone: "UTC".into(),
        })?;
        let existing_series = Model {
            id: series_id,
            coaching_relationship_id: Id::new_v4(),
            rule: stored_rule,
            ical_sequence: 4,
            created_by_user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let future_session = coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: existing_series.coaching_relationship_id,
            coaching_session_series_id: Some(series_id),
            ical_sequence: 0,
            ical_recurrence_id: None,
            collab_document_name: None,
            date: start_at + chrono::Duration::days(7),
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
            notice_given_at: chrono::Utc::now().into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![coach_row("UTC")]])
            .append_query_results(vec![vec![existing_series.clone()]])
            .append_query_results(vec![vec![future_session.clone()]])
            .into_connection();

        let rescheduled = reschedule(
            &db,
            &test_config(),
            series_id,
            Id::new_v4(),
            start_at,
            weekly_rule_count(3),
            None,
        )
        .await?;

        assert!(!rescheduled.changed, "an identical rule changed nothing");
        assert_eq!(rescheduled.series.ical_sequence, 4, "SEQUENCE not burned");
        assert_eq!(
            rescheduled.sessions,
            vec![future_session],
            "the surviving sessions are returned, not recreated ones"
        );

        let sql: Vec<String> = db
            .into_transaction_log()
            .iter()
            .flat_map(|txn| txn.statements())
            .map(|stmt| stmt.sql.clone())
            .collect();
        assert!(
            !sql.iter()
                .any(|s| s.contains("DELETE") || s.contains("UPDATE") || s.contains("INSERT")),
            "a no-op reschedule must not write: {sql:?}"
        );
        Ok(())
    }

    /// The stored rule is compared as a parsed `SeriesRule`, not as a JSON blob. Here it
    /// carries an extra key that serde ignores, so the rules mean the same thing while
    /// their JSON differs. A blob comparison would call this a change and destroy every
    /// future session to rebuild an identical schedule.
    #[tokio::test]
    async fn reschedule_compares_rules_by_meaning_not_by_json() -> Result<(), Error> {
        let series_id = Id::new_v4();
        let now = chrono::Utc::now();
        let start_at = start();
        let mut stored_rule = serde_json::to_value(SeriesRule {
            start_at,
            recurrence: weekly_rule_count(3),
            duration_minutes: 60,
            timezone: "UTC".into(),
        })?;
        // A field written by some other version of this struct.
        stored_rule["legacy_field"] = serde_json::json!("ignored");

        let existing_series = Model {
            id: series_id,
            coaching_relationship_id: Id::new_v4(),
            rule: stored_rule,
            ical_sequence: 9,
            created_by_user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let future_session = coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: existing_series.coaching_relationship_id,
            coaching_session_series_id: Some(series_id),
            ical_sequence: 0,
            ical_recurrence_id: None,
            collab_document_name: None,
            date: start_at + chrono::Duration::days(7),
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
            notice_given_at: chrono::Utc::now().into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![coach_row("UTC")]])
            .append_query_results(vec![vec![existing_series.clone()]])
            .append_query_results(vec![vec![future_session]])
            .into_connection();

        let rescheduled = reschedule(
            &db,
            &test_config(),
            series_id,
            Id::new_v4(),
            start_at,
            weekly_rule_count(3),
            None,
        )
        .await?;

        assert!(
            !rescheduled.changed,
            "an ignored extra key does not change what the rule means"
        );
        assert_eq!(rescheduled.series.ical_sequence, 9, "SEQUENCE not burned");
        Ok(())
    }

    /// A reschedule with a `start_at` in the past is rejected before any DB
    /// access — re-materializing the past on top of surviving past sessions
    /// would corrupt the series timeline.
    #[tokio::test]
    async fn reschedule_rejects_past_start_at() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let past = NaiveDate::from_ymd_opt(2000, 1, 1)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();

        let result = reschedule(
            &db,
            &test_config(),
            Id::new_v4(),
            Id::new_v4(),
            past,
            weekly_rule_count(3),
            None,
        )
        .await;

        let err = result.expect_err("a past start_at must be rejected");
        assert!(
            matches!(
                err.error_kind,
                crate::error::DomainErrorKind::Validation(ref m) if m.contains("past")
            ),
            "expected a past-start validation error, got {err:?}"
        );
    }

    /// Series delete: future sessions are loaded, locked, bulk-deleted, and
    /// then the series row itself is removed. Past sessions are not touched.
    /// Tiptap is never reached because no future session has a
    /// collab doc.
    #[tokio::test]
    async fn delete_with_future_sessions_clears_future_and_series_rows() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let series_id = Id::new_v4();
        let now = chrono::Utc::now();

        let make_session = |date: NaiveDateTime| coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: Some(series_id),
            ical_sequence: 0,
            ical_recurrence_id: None,
            collab_document_name: None,
            date,
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
            notice_given_at: chrono::Utc::now().into(),
        };

        let future_sessions = vec![
            make_session(start() + chrono::Duration::days(7)),
            make_session(start() + chrono::Duration::days(14)),
        ];

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // find_future_sessions_by_series_id → 2 future rows
            .append_query_results(vec![future_sessions.clone()])
            // storage_keys_for_sessions → no images to strand
            .append_query_results(vec![Vec::<coaching_session_images::Model>::new()])
            // BEGIN
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // acquire_advisory_lock × 2
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // re-read of the future sessions, now under the locks
            .append_query_results(vec![future_sessions.clone()])
            // bulk_delete_by_ids → DELETE
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 2,
            }])
            // coaching_session_series::delete → DELETE
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // COMMIT
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        delete_with_future_sessions(&db, &test_config(), series_id).await?;
        Ok(())
    }

    /// The pre-transaction read happens before the advisory locks, so an individual
    /// reschedule committing in between bumps an `ical_sequence` that snapshot cannot see.
    /// A cancellation built off the stale number reuses a `SEQUENCE` the client already
    /// applied and is silently dropped, so the rows have to be read again under the locks.
    #[tokio::test]
    async fn reschedule_rereads_the_replaced_sessions_under_the_locks() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let series_id = Id::new_v4();
        let now = chrono::Utc::now();
        let session_id = Id::new_v4();

        let existing_series = Model {
            id: series_id,
            coaching_relationship_id: relationship_id,
            rule: serde_json::to_value(SeriesRule {
                start_at: start(),
                recurrence: weekly_rule_count(1),
                duration_minutes: 60,
                timezone: "UTC".into(),
            })?,
            ical_sequence: 0,
            created_by_user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let session = |ical_sequence: i32| coaching_sessions::Model {
            id: session_id,
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: Some(series_id),
            ical_sequence,
            ical_recurrence_id: None,
            collab_document_name: None,
            date: start() + chrono::Duration::days(7),
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
            notice_given_at: chrono::Utc::now().into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![coach_row("UTC")]])
            .append_query_results(vec![vec![existing_series.clone()]])
            // The pre-lock snapshot still shows SEQUENCE 1.
            .append_query_results(vec![vec![session(1)]])
            // storage_keys_for_sessions → no images to strand
            .append_query_results(vec![Vec::<coaching_session_images::Model>::new()])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // Under the locks it reads 4: an individual reschedule got in between.
            .append_query_results(vec![vec![session(4)]])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results(vec![vec![existing_series.clone()]])
            .append_query_results(vec![vec![session(0)]])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        reschedule(
            &db,
            &test_config(),
            series_id,
            Id::new_v4(),
            start(),
            weekly_rule_count(1),
            Some(Duration::from_minutes_unchecked(60)),
        )
        .await?;

        // Two selects against coaching_sessions: one before the locks, one after. Without
        // the second, the cancellation would be built from a SEQUENCE that is already spent.
        let selects = db
            .into_transaction_log()
            .iter()
            .flat_map(|txn| txn.statements())
            .filter(|statement| {
                statement.sql.contains("SELECT")
                    && statement
                        .sql
                        .contains(r#"FROM "refactor_platform"."coaching_sessions""#)
            })
            .count();
        assert_eq!(
            selects, 2,
            "the replaced sessions must be read again once the locks are held"
        );

        Ok(())
    }

    #[tokio::test]
    async fn delete_swallows_tiptap_cleanup_failure() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let series_id = Id::new_v4();
        let now = chrono::Utc::now();

        // One future session carrying a collab doc → cleanup is attempted.
        let future_session = coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: Some(series_id),
            ical_sequence: 0,
            ical_recurrence_id: None,
            collab_document_name: Some("doc-to-clean-up".to_string()),
            date: start() + chrono::Duration::days(7),
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
            notice_given_at: chrono::Utc::now().into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // find_future_sessions_by_series_id → 1 future row (with a doc)
            .append_query_results(vec![vec![future_session.clone()]])
            // storage_keys_for_sessions → no images to strand
            .append_query_results(vec![Vec::<coaching_session_images::Model>::new()])
            // BEGIN
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // acquire_advisory_lock × 1
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // re-read of the future sessions, now under the locks
            .append_query_results(vec![vec![future_session.clone()]])
            // bulk_delete_by_ids → DELETE
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // coaching_session_series::delete → DELETE
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            // COMMIT
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        // Tiptap is unreachable, so cleanup fails internally — but the call
        // returns Ok, proving the failure is swallowed rather than propagated.
        delete_with_future_sessions(&db, &test_config(), series_id).await?;
        Ok(())
    }

    fn test_config() -> Config {
        Config::from_args([
            "test",
            "--tiptap-auth-key=test-auth-key",
            "--tiptap-url=http://127.0.0.1:0",
        ])
    }

    fn exec_ok() -> MockExecResult {
        MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }
    }

    /// A weekly series of `n` sessions, one on each slot of its stored rule.
    fn following_fixture(n: u32) -> (Model, SeriesRule, Vec<coaching_sessions::Model>) {
        let now = chrono::Utc::now();
        let rule = SeriesRule {
            start_at: start(),
            recurrence: weekly_rule_count(n),
            duration_minutes: 60,
            timezone: "UTC".into(),
        };
        let series = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            rule: serde_json::to_value(&rule).unwrap(),
            ical_sequence: 3,
            created_by_user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        };
        let sessions = rule
            .slots()
            .unwrap()
            .into_iter()
            .map(|slot| coaching_sessions::Model {
                id: Id::new_v4(),
                coaching_relationship_id: series.coaching_relationship_id,
                coaching_session_series_id: Some(series.id),
                ical_sequence: 1,
                ical_recurrence_id: Some(slot),
                collab_document_name: None,
                date: slot,
                duration_minutes: 60,
                title: None,
                meeting_url: None,
                provider: None,
                created_at: now.into(),
                updated_at: now.into(),
                hydrated_at: None,
                notice_given_at: now.into(),
            })
            .collect();
        (series, rule, sessions)
    }

    fn written_sql(db: sea_orm::DatabaseConnection) -> Vec<String> {
        db.into_transaction_log()
            .iter()
            .flat_map(|txn| txn.statements())
            .map(|stmt| stmt.sql.clone())
            .collect()
    }

    fn writes(sql: &[String]) -> Vec<&String> {
        sql.iter()
            .filter(|s| s.contains("DELETE") || s.contains("UPDATE") || s.contains("INSERT"))
            .collect()
    }

    #[tokio::test]
    async fn reschedule_following_onto_the_slots_it_already_holds_writes_nothing(
    ) -> Result<(), Error> {
        let (series, _, sessions) = following_fixture(3);
        let anchor = sessions[1].clone();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![series.clone()]])
            .append_query_results(vec![vec![coach_row("UTC")]])
            .append_query_results(vec![sessions.clone()])
            .append_exec_results(vec![exec_ok(), exec_ok(), exec_ok()])
            .append_query_results(vec![sessions.clone()])
            .into_connection();

        let rescheduled = reschedule_following(
            &db,
            &test_config(),
            series.id,
            Id::new_v4(),
            FollowingReschedule {
                coaching_session_id: anchor.id,
                start_at: anchor.date,
                duration: None,
                recurrence: None,
            },
        )
        .await?;

        assert!(!rescheduled.changed);
        assert_eq!(rescheduled.series.ical_sequence, 3, "SEQUENCE not burned");
        assert_eq!(rescheduled.sessions, sessions[1..].to_vec());
        let sql = written_sql(db);
        assert!(writes(&sql).is_empty(), "a no-op must not write: {sql:?}");
        Ok(())
    }

    #[tokio::test]
    async fn reschedule_following_refuses_to_drop_leftovers_with_content() {
        let (series, _, sessions) = following_fixture(3);
        let with_note = sessions[2].id;
        let noted = |id: Id| {
            std::collections::BTreeMap::from([("coaching_session_id", sea_orm::Value::from(id))])
        };
        let none = Vec::<std::collections::BTreeMap<&str, sea_orm::Value>>::new;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![series.clone()]])
            .append_query_results(vec![vec![coach_row("UTC")]])
            .append_query_results(vec![sessions.clone()])
            .append_exec_results(vec![exec_ok(), exec_ok(), exec_ok(), exec_ok()])
            .append_query_results(vec![sessions.clone()])
            // notes, actions, agreements, topics
            .append_query_results(vec![vec![noted(with_note)]])
            .append_query_results(vec![none(), none(), none()])
            .into_connection();

        let err = reschedule_following(
            &db,
            &test_config(),
            series.id,
            Id::new_v4(),
            FollowingReschedule {
                coaching_session_id: sessions[0].id,
                start_at: sessions[0].date,
                duration: None,
                recurrence: Some(weekly_rule_count(1)),
            },
        )
        .await
        .unwrap_err();

        let DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Conflict {
            details: Some(details),
            ..
        })) = err.error_kind
        else {
            panic!("expected a conflict, got {err:?}");
        };
        assert_eq!(details["reason"], "sessions_have_content");
        assert_eq!(
            details["coaching_session_ids"],
            serde_json::json!([with_note])
        );
        let sql = written_sql(db);
        assert!(writes(&sql).is_empty(), "a refusal must not write: {sql:?}");
    }

    #[tokio::test]
    async fn reschedule_following_splits_off_a_new_series_for_the_moved_sessions(
    ) -> Result<(), Error> {
        let (series, rule, sessions) = following_fixture(3);
        let anchor = sessions[1].clone();
        let new_start = anchor.date + chrono::Duration::hours(1);
        let now = chrono::Utc::now();
        let shortened = Model {
            ical_sequence: 4,
            ..series.clone()
        };
        let new_series = Model {
            id: Id::new_v4(),
            ical_sequence: 0,
            created_at: now.into(),
            updated_at: now.into(),
            ..series.clone()
        };
        let moved = |session: &coaching_sessions::Model, slot| coaching_sessions::Model {
            coaching_session_series_id: Some(new_series.id),
            date: slot,
            ical_recurrence_id: Some(slot),
            ical_sequence: session.ical_sequence + 1,
            ..session.clone()
        };
        let moved_rows = vec![
            moved(&sessions[1], new_start),
            moved(&sessions[2], new_start + chrono::Duration::weeks(1)),
        ];

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![series.clone()]])
            .append_query_results(vec![vec![coach_row("UTC")]])
            .append_query_results(vec![sessions.clone()])
            .append_exec_results(vec![exec_ok(), exec_ok(), exec_ok()])
            .append_query_results(vec![sessions.clone()])
            // update_rule on the old series, then the new series' insert
            .append_query_results(vec![vec![shortened.clone()]])
            .append_query_results(vec![vec![new_series.clone()]])
            // move_to_slot × 2
            .append_query_results(vec![vec![moved_rows[0].clone()]])
            .append_query_results(vec![vec![moved_rows[1].clone()]])
            .append_exec_results(vec![exec_ok()])
            .into_connection();

        let rescheduled = reschedule_following(
            &db,
            &test_config(),
            series.id,
            Id::new_v4(),
            FollowingReschedule {
                coaching_session_id: anchor.id,
                start_at: new_start,
                duration: None,
                recurrence: None,
            },
        )
        .await?;

        assert!(rescheduled.changed);
        assert_eq!(rescheduled.series.id, new_series.id);
        assert_eq!(rescheduled.sessions, moved_rows);

        let log = db.into_transaction_log();
        let statements: Vec<_> = log.iter().flat_map(|txn| txn.statements()).collect();
        let written: Vec<_> = statements
            .iter()
            .filter(|s| s.sql.contains("UPDATE") || s.sql.contains("INSERT"))
            .collect();
        assert_eq!(written.len(), 4, "{written:?}");

        let until = (anchor.date - chrono::Duration::seconds(1))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        assert!(written[0]
            .sql
            .starts_with(r#"UPDATE "refactor_platform"."coaching_session_series""#));
        assert!(
            format!("{:?}", written[0].values).contains(&until),
            "the old rule ends just before the anchor: {:?}",
            written[0].values
        );
        assert!(written[1]
            .sql
            .starts_with(r#"INSERT INTO "refactor_platform"."coaching_session_series""#));
        // The new series starts at the moved session, not at the old rule's first slot.
        let old_start = rule.start_at.format("%Y-%m-%dT%H:%M").to_string();
        let new_series_values = format!("{:?}", written[1].values);
        assert!(new_series_values.contains(&new_start.format("%Y-%m-%dT%H:%M").to_string()));
        assert!(!new_series_values.contains(&old_start));
        for move_statement in &written[2..] {
            assert!(move_statement
                .sql
                .starts_with(r#"UPDATE "refactor_platform"."coaching_sessions""#));
            assert!(
                format!("{:?}", move_statement.values).contains(&new_series.id.to_string()),
                "sessions move onto the new series: {:?}",
                move_statement.values
            );
        }
        assert!(
            !statements.iter().any(|s| s.sql.contains("DELETE")),
            "a split that keeps every session deletes nothing"
        );
        Ok(())
    }
}
