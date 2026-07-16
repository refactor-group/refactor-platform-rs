use chrono::NaiveDateTime;
use serde::Deserialize;
use utoipa::ToSchema;

use domain::{coaching_session::Recurrence, Id};

/// Request body for `POST /coaching_session_series`. Creates the series and
/// materializes its sessions in one transaction. Each materialized session
/// shares the same duration; omitting `duration_minutes` triggers the BE
/// defaulting cascade and the resolved value is then persisted on the
/// stored rule.
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct CreateParams {
    pub(crate) coaching_relationship_id: Id,
    pub(crate) start_at: NaiveDateTime,
    pub(crate) recurrence: Recurrence,
    /// Session duration in minutes (1..=480). Omit to use the coach's stored
    /// `default_coaching_session_duration_minutes`.
    pub(crate) duration_minutes: Option<i16>,
}

/// Request body for `PUT /coaching_session_series/:id`. Replaces the rule
/// entirely (no partial updates) and re-materializes future sessions; past
/// sessions are not touched.
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct RescheduleParams {
    pub(crate) start_at: NaiveDateTime,
    pub(crate) recurrence: Recurrence,
    /// Session duration in minutes (1..=480). Omit to use the coach's stored
    /// `default_coaching_session_duration_minutes`.
    pub(crate) duration_minutes: Option<i16>,
}
