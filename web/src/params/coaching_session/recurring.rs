use chrono::NaiveDateTime;
use serde::Deserialize;
use utoipa::ToSchema;

use domain::{coaching_session::Recurrence, Id};

/// Request body for `POST /coaching_sessions/recurring`. Expands the
/// `recurrence` rule starting at `start_at` into individual sessions on
/// `coaching_relationship_id`.
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct CreateRecurringParams {
    pub(crate) coaching_relationship_id: Id,
    pub(crate) start_at: NaiveDateTime,
    pub(crate) recurrence: Recurrence,
}
