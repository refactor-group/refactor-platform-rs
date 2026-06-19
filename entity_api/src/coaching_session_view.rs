use super::error::{EntityApiErrorKind, Error};
use entity::Id;
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};
use serde::Serialize;
use utoipa::ToSchema;

/// Result of marking a session viewed: the new marker and the value it had immediately before.
#[derive(Debug, Clone, FromQueryResult, Serialize, ToSchema)]
pub struct MarkViewed {
    pub previous_last_viewed_at: Option<DateTimeWithTimeZone>,
    pub last_viewed_at: DateTimeWithTimeZone,
}

/// Upsert the caller's view marker to now() and return the prior value atomically.
/// At most one row per (user_id, coaching_session_id) (enforced by the unique constraint).
///
/// A CTE snapshots the pre-existing row against the statement-start snapshot, then the
/// INSERT ... ON CONFLICT DO UPDATE advances the marker; the CTE yields the PRIOR value
/// (NULL on first view).
pub async fn mark_viewed(
    db: &impl ConnectionTrait,
    coaching_session_id: Id,
    user_id: Id,
) -> Result<MarkViewed, Error> {
    let stmt = Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"WITH prev AS (
               SELECT last_viewed_at
               FROM refactor_platform.coaching_session_views
               WHERE coaching_session_id = $1 AND user_id = $2
           )
           INSERT INTO refactor_platform.coaching_session_views
               (coaching_session_id, user_id, last_viewed_at, created_at, updated_at)
           VALUES ($1, $2, NOW(), NOW(), NOW())
           ON CONFLICT (user_id, coaching_session_id)
           DO UPDATE SET last_viewed_at = NOW(), updated_at = NOW()
           RETURNING last_viewed_at, (SELECT last_viewed_at FROM prev) AS previous_last_viewed_at"#,
        [coaching_session_id.into(), user_id.into()],
    );
    MarkViewed::find_by_statement(stmt)
        .one(db)
        .await?
        .ok_or_else(|| Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        })
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "coaching_session_view_tests.rs"]
mod tests;
