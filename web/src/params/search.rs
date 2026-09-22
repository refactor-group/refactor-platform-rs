//! `GET /search` request parameters and their compilation into a validated
//! `domain::search::Spec`. Every 400-class decision lives here, in the web
//! layer — the domain never raises boundary-validation errors (which would
//! surface as 422 through the domain error chain).

use std::str::FromStr;

use chrono::{NaiveDate, TimeZone};
use chrono_tz::Tz;
use domain::search::{Cursor, Filters, GoalFilter, HitType, Spec, TimeRange};
use domain::status::Status;
use domain::topic_status::Status as TopicStatus;
use domain::Id;
use sea_orm::entity::prelude::DateTimeWithTimeZone;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::error::WebErrorKind;
use crate::Error;

const MIN_QUERY_CHARS: usize = 2;
const MAX_QUERY_CHARS: usize = 256;
const DEFAULT_LIMIT: u16 = 25;
const MAX_LIMIT: u16 = 100;

/// Search mode. `semantic` and `hybrid` are advertised from day one and
/// rejected with `mode_unavailable` until their phases ship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    #[default]
    Keyword,
    Semantic,
    Hybrid,
}

/// Actions-by-goal-linkage filter. Mirrors the `assignee_filter` vocabulary
/// precedent on `GET /users/{id}/actions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GoalFilterParam {
    #[default]
    All,
    Linked,
    Unlinked,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    /// Query text (required, >= 2 chars after trim, silently truncated at 256).
    pub(crate) q: String,
    /// Comma-separated entity types; unknown token → 400, valid-but-not-
    /// permitted (or not yet active) → silently dropped.
    pub(crate) types: Option<String>,
    pub(crate) organization_id: Option<Id>,
    pub(crate) coaching_relationship_id: Option<Id>,
    pub(crate) user_id: Option<Id>,
    pub(crate) coaching_session_id: Option<Id>,
    pub(crate) goal_id: Option<Id>,
    pub(crate) goal_filter: Option<GoalFilterParam>,
    pub(crate) status: Option<Status>,
    pub(crate) topic_status: Option<TopicStatus>,
    pub(crate) created_from: Option<NaiveDate>,
    pub(crate) created_to: Option<NaiveDate>,
    pub(crate) updated_from: Option<NaiveDate>,
    pub(crate) updated_to: Option<NaiveDate>,
    /// IANA timezone for the date windows; defaults to UTC.
    pub(crate) tz: Option<String>,
    pub(crate) limit: Option<u16>,
    pub(crate) cursor: Option<String>,
    pub(crate) mode: Option<Mode>,
    /// Hybrid-mode fusion weight; 400 until PR 7 ships `mode=hybrid`.
    pub(crate) keyword_weight: Option<f32>,
}

fn invalid(error: &'static str, message: String) -> Error {
    Error::Web(WebErrorKind::InvalidParam { error, message })
}

impl IndexParams {
    /// Validate and compile into the domain's `Spec`. Every rejection is a
    /// structured 400 with a stable discriminator.
    pub(crate) fn compile(self) -> Result<Spec, Error> {
        // Truncate silently (clamp precedent), preserving the raw tail so the
        // final-token prefix rule can still see a trailing-whitespace opt-out.
        let q_raw: String = self.q.chars().take(MAX_QUERY_CHARS).collect();
        let query = q_raw.trim().to_string();
        if query.chars().count() < MIN_QUERY_CHARS {
            return Err(invalid(
                "query_too_short",
                format!("`q` must be at least {MIN_QUERY_CHARS} characters after trimming."),
            ));
        }

        if self.mode.unwrap_or_default() != Mode::Keyword {
            return Err(invalid(
                "mode_unavailable",
                "Only `mode=keyword` is available; `semantic` and `hybrid` are reserved for later phases.".to_string(),
            ));
        }
        if let Some(w) = self.keyword_weight {
            if !(0.0..=1.0).contains(&w) {
                return Err(invalid(
                    "keyword_weight_out_of_range",
                    format!("`keyword_weight` must be within [0.0, 1.0], got {w}."),
                ));
            }
            // Range-valid but meaningless outside mode=hybrid (unreachable
            // until PR 7, so any surviving keyword_weight is contradictory).
            return Err(invalid(
                "contradictory_params",
                "`keyword_weight` only applies to `mode=hybrid`.".to_string(),
            ));
        }

        let goal_filter = self.goal_filter.unwrap_or_default();
        if goal_filter == GoalFilterParam::Unlinked && self.goal_id.is_some() {
            return Err(invalid(
                "contradictory_params",
                "`goal_filter=unlinked` cannot be combined with `goal_id`.".to_string(),
            ));
        }

        let types = resolve_types(self.types.as_deref())?;

        let tz = match self.tz.as_deref() {
            Some(raw) => Tz::from_str(raw)
                .map_err(|_| Error::Web(WebErrorKind::InvalidTimezone(raw.to_string())))?,
            None => Tz::UTC,
        };

        let cursor = self
            .cursor
            .as_deref()
            .map(|raw| {
                Cursor::decode(raw).map_err(|_| {
                    invalid(
                        "malformed_cursor",
                        "`cursor` is not a cursor issued by this endpoint.".to_string(),
                    )
                })
            })
            .transpose()?;

        Ok(Spec {
            q_raw,
            query,
            types,
            coaching_relationship_id: self.coaching_relationship_id,
            filters: Filters {
                organization_id: self.organization_id,
                user_id: self.user_id,
                coaching_session_id: self.coaching_session_id,
                goal_id: self.goal_id,
                goal_filter: match goal_filter {
                    GoalFilterParam::All => GoalFilter::All,
                    GoalFilterParam::Linked => GoalFilter::Linked,
                    GoalFilterParam::Unlinked => GoalFilter::Unlinked,
                },
                status: self.status,
                topic_status: self.topic_status,
                created: day_range(self.created_from, self.created_to, tz),
                updated: day_range(self.updated_from, self.updated_to, tz),
                participant_relationship_ids: None,
            },
            limit: self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
            cursor,
        })
    }
}

/// The full `types` vocabulary, present from day one. Tokens for types whose
/// searcher has not shipped (`notes` PR 5, `transcripts` PR 2) or that the
/// caller may not search (`users`, `organizations` — role-gated in PR 3) are
/// known-but-inactive: never a 400, silently dropped.
const INACTIVE_TYPE_TOKENS: [&str; 4] = ["notes", "transcripts", "users", "organizations"];

fn parse_type_token(token: &str) -> Result<Option<HitType>, Error> {
    match token {
        "coaching_sessions" => Ok(Some(HitType::CoachingSession)),
        "goals" => Ok(Some(HitType::Goal)),
        "actions" => Ok(Some(HitType::Action)),
        "agreements" => Ok(Some(HitType::Agreement)),
        "topics" => Ok(Some(HitType::Topic)),
        t if INACTIVE_TYPE_TOKENS.contains(&t) => Ok(None),
        unknown => Err(invalid(
            "unknown_type",
            format!("`{unknown}` is not a searchable type."),
        )),
    }
}

/// Omitted param → every active type. Supplied → the mapped subset, which may
/// legitimately end up empty (every token dropped) and then searches nothing.
fn resolve_types(raw: Option<&str>) -> Result<Vec<HitType>, Error> {
    let Some(raw) = raw else {
        return Ok(vec![
            HitType::Action,
            HitType::Agreement,
            HitType::CoachingSession,
            HitType::Goal,
            HitType::Topic,
        ]);
    };
    let mut types: Vec<HitType> = raw
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(parse_type_token)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    types.sort();
    types.dedup();
    Ok(types)
}

/// Half-open `[from, to + 1 day)` window over calendar days in `tz`.
fn day_range(from: Option<NaiveDate>, to: Option<NaiveDate>, tz: Tz) -> TimeRange {
    TimeRange {
        from: from.map(|d| day_start(d, tz)),
        to_exclusive: to
            .and_then(|d| d.succ_opt())
            .map(|next| day_start(next, tz)),
    }
}

/// Local midnight of `date` in `tz`, as an absolute timestamp. On the rare DST
/// gap where local midnight does not exist, the earliest valid instant of the
/// day is used; on an ambiguous midnight, the earlier one.
fn day_start(date: NaiveDate, tz: Tz) -> DateTimeWithTimeZone {
    let midnight = date.and_hms_opt(0, 0, 0).unwrap_or_default();
    tz.from_local_datetime(&midnight)
        .earliest()
        .unwrap_or_else(|| {
            tz.from_local_datetime(&(midnight + chrono::Duration::hours(1)))
                .earliest()
                .unwrap_or_else(|| tz.from_utc_datetime(&midnight))
        })
        .fixed_offset()
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
