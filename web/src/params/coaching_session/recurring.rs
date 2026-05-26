use chrono::NaiveDateTime;
use serde::Deserialize;
use utoipa::ToSchema;

use domain::{coaching_session::Recurrence, Id};

/// Request body for `POST /coaching_sessions/recurring`. Expands the
/// `recurrence` rule starting at `start_at` into individual sessions on
/// `coaching_relationship_id`. Each materialized session uses the same
/// duration; omitting `duration_minutes` triggers the BE defaulting cascade.
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct CreateRecurringParams {
    pub(crate) coaching_relationship_id: Id,
    pub(crate) start_at: NaiveDateTime,
    pub(crate) recurrence: Recurrence,
    /// Session duration in minutes (1..=480). Omit to use the coach's stored
    /// `default_coaching_session_duration_minutes`.
    pub(crate) duration_minutes: Option<i16>,
}
