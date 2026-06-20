//! Server-side composition of a coaching session's `display_title`.
//!
//! `display_title` is a read-time projection, not stored state: it composes the
//! fallback chain `human title -> first topic body -> first goal title` and is
//! `null` when no tier yields text (the server invents no placeholder). It reuses
//! the canonical topic ordering and the `include=goal` source so the composed
//! title matches the single-session page by construction.

use super::error::Error;
use entity::{coaching_session_topics, coaching_sessions, Id};
use sea_orm::{entity::prelude::*, ConnectionTrait, QueryOrder};
use std::collections::HashMap;
use utoipa::ToSchema;

/// Relationship-scoped list read shape: a base session plus its composed
/// `display_title`. Unlike the enriched read it carries no caller-scoped fields
/// (e.g. `viewer_last_viewed_at`), so it is safe on the participant-shared list.
#[derive(Debug, Clone, serde::Serialize, ToSchema)]
#[schema(as = domain::coaching_session::SessionWithDisplayTitle)]
pub struct SessionWithDisplayTitle {
    #[serde(flatten)]
    pub session: coaching_sessions::Model,
    // Composed fallback title; null when none derive. Always present.
    pub display_title: Option<String>,
}

/// Compose a session's display title from the fallback chain:
/// human title -> first topic body -> first goal title. Empty / whitespace-only
/// inputs are treated as absent so a blank tier falls through to the next.
/// Returns `None` when no tier yields text.
pub(crate) fn compose_display_title(
    session_title: Option<&str>,
    first_topic_body: Option<&str>,
    first_goal_title: Option<&str>,
) -> Option<String> {
    [session_title, first_topic_body, first_goal_title]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(str::to_owned)
}

/// First (drag-order) live topic body per session, keyed by session id.
///
/// Reuses the canonical ordering + soft-delete filter of
/// [`super::coaching_session_topic::find_by_coaching_session_id`], so it inherits
/// v5 move/defer parenting (`CoachingSessionId` = this session) and v6 soft-delete
/// exclusion (`DeletedAt IS NULL`). Results are globally ordered, so the first row
/// seen per session is that session's drag-order-first topic.
async fn batch_load_first_topic_bodies(
    db: &impl ConnectionTrait,
    session_ids: &[Id],
) -> Result<HashMap<Id, String>, Error> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut map: HashMap<Id, String> = HashMap::new();
    for topic in coaching_session_topics::Entity::find()
        .filter(
            coaching_session_topics::Column::CoachingSessionId.is_in(session_ids.iter().copied()),
        )
        .filter(coaching_session_topics::Column::DeletedAt.is_null())
        .order_by_asc(coaching_session_topics::Column::DisplayOrder)
        .order_by_asc(coaching_session_topics::Column::CreatedAt)
        .all(db)
        .await?
    {
        // Keep only the first (canonical-order) topic per session. Bodies are
        // stored raw (even if empty/whitespace); compose_display_title is the
        // single authority on emptiness and trims/falls through as needed.
        map.entry(topic.coaching_session_id).or_insert(topic.body);
    }
    Ok(map)
}

/// First linked goal title per session, keyed by session id. Reuses the same
/// grouped-goals source as `include=goal` so the title tier matches what that
/// include returns. Skips goals whose title is absent.
async fn batch_load_first_goal_titles(
    db: &impl ConnectionTrait,
    session_ids: &[Id],
) -> Result<HashMap<Id, String>, Error> {
    let grouped =
        super::coaching_session_goal::find_goals_grouped_by_session_ids(db, session_ids).await?;

    Ok(grouped
        .into_iter()
        .filter_map(|(session_id, goals)| {
            // First goal that actually has a title (a leading title-less goal must
            // not drop the tier). Goals arrive in deterministic order from
            // find_goals_grouped_by_session_ids. compose_display_title trims any
            // blank result, so emptiness policy stays in one place.
            goals
                .into_iter()
                .find_map(|g| g.title)
                .map(|title| (session_id, title))
        })
        .collect())
}

/// Compose display titles for every passed session from its own title plus the
/// first-topic-body and first-goal-title tiers. Every session id is present in
/// the result (value `None` when no tier derives).
pub async fn batch_load_display_titles(
    db: &impl ConnectionTrait,
    sessions: &[coaching_sessions::Model],
) -> Result<HashMap<Id, Option<String>>, Error> {
    let session_ids: Vec<Id> = sessions.iter().map(|s| s.id).collect();
    let first_topic_bodies = batch_load_first_topic_bodies(db, &session_ids).await?;
    let first_goal_titles = batch_load_first_goal_titles(db, &session_ids).await?;

    Ok(sessions
        .iter()
        .map(|s| {
            let title = compose_display_title(
                s.title.as_deref(),
                first_topic_bodies.get(&s.id).map(String::as_str),
                first_goal_titles.get(&s.id).map(String::as_str),
            );
            (s.id, title)
        })
        .collect())
}

#[cfg(test)]
#[path = "coaching_session_display_title_tests.rs"]
mod tests;

#[cfg(all(test, feature = "mock"))]
#[path = "coaching_session_display_title_mock_tests.rs"]
mod mock_tests;
