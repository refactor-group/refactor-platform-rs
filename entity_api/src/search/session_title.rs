//! Post-merge session-title resolution for search hits.
//!
//! Search-owned, unlike the list endpoints' nullable `display_title`: the
//! composed title falls back to "Coaching session — YYYY-MM-DD" (naive session
//! date, identical for every caller), so a resolved title is never empty.

use std::collections::HashMap;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use crate::coaching_session_display_title::batch_load_display_titles;
use crate::error::Error;
use entity::{coaching_sessions, Id};

/// Never-empty titles for the given sessions, keyed by session id. Ids with no
/// matching session are absent from the map, so the caller leaves those hits
/// untouched.
pub async fn resolve_session_titles(
    db: &DatabaseConnection,
    session_ids: &[Id],
) -> Result<HashMap<Id, String>, Error> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let sessions = coaching_sessions::Entity::find()
        .filter(coaching_sessions::Column::Id.is_in(session_ids.iter().copied()))
        .all(db)
        .await?;
    let composed = batch_load_display_titles(db, &sessions).await?;

    Ok(sessions
        .iter()
        .map(|s| {
            let title = composed
                .get(&s.id)
                .cloned()
                .flatten()
                .unwrap_or_else(|| format!("Coaching session — {}", s.date.format("%Y-%m-%d")));
            (s.id, title)
        })
        .collect())
}

#[cfg(all(test, feature = "mock"))]
#[path = "session_title_mock_tests.rs"]
mod mock_tests;
