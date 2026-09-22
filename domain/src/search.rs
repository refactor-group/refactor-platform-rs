//! Search orchestration: scope derivation, one-per-request visibility
//! resolution, bounded searcher fan-out, merge/rank/paginate, and post-merge
//! hydration (display titles and snippets for exactly the returned page).

use futures::stream::{self, StreamExt};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter, QuerySelect,
    QueryTrait,
};
use serde::Serialize;
use utoipa::ToSchema;

use crate::coaching_relationships::{self, visible_to, Scope};
use crate::coaching_sessions;
use crate::error::Error;
use crate::users::{self, Role};
use crate::Id;
use entity_api::coaching_session_display_title::batch_load_display_titles;
use entity_api::search::{hydrate_snippets, searchers, Request};

pub use entity_api::search::{
    ActionHit, AgreementHit, Core, Cursor, CursorDecodeError, Filters, GoalFilter, GoalHit, Hit,
    HitType, SessionHit, TimeRange, TopicHit,
};

/// How many searchers run concurrently. Each holds a pool connection while in
/// flight, and search-as-you-type multiplies requests — the bound caps
/// per-request connection draw (see the fan-out decision in the plan).
const SEARCHER_CONCURRENCY: usize = 3;

/// A validated, compiled search request, produced by the web layer from
/// `IndexParams`. Every 400-class decision has already happened.
#[derive(Debug, Clone)]
pub struct Spec {
    /// Length-clamped but untrimmed query text — the final-token prefix rule
    /// inspects the raw tail.
    pub q_raw: String,
    /// The trimmed query echoed back in [`Results::query`].
    pub query: String,
    /// The resolved types to search. The web layer always materializes this
    /// (omitted param = every active type), so an empty list genuinely means
    /// "nothing to search" — e.g. every requested type was silently dropped —
    /// and yields an empty page, not an unfiltered search.
    pub types: Vec<HitType>,
    /// The `coaching_relationship_id` filter param; folded into the resolved
    /// relationship id set (intersect-only).
    pub coaching_relationship_id: Option<Id>,
    /// `participant_relationship_ids` must be `None`; this function compiles it.
    pub filters: Filters,
    /// Post-clamp page size (1..=100).
    pub limit: u16,
    pub cursor: Option<Cursor>,
}

/// The `data` payload of `GET /search`.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = domain::search::Results)]
pub struct Results {
    /// Trimmed, post-clamp — what was actually searched.
    pub query: String,
    /// Post-clamp page size.
    pub limit: u16,
    /// Ordered by `(score DESC, type ASC, id ASC)`.
    pub hits: Vec<Hit>,
    pub next_cursor: Option<String>,
}

/// Derive the caller's visibility scope from their preloaded roles.
/// Zero queries — roles arrive on `AuthenticatedUser`.
pub fn scope_for(user: &users::Model) -> Scope {
    Scope {
        user_id: user.id,
        is_super_admin: user
            .roles
            .iter()
            .any(|r| r.role == Role::SuperAdmin && r.organization_id.is_none()),
        admin_org_ids: user
            .roles
            .iter()
            .filter(|r| r.role == Role::Admin)
            .filter_map(|r| r.organization_id)
            .collect(),
        member_org_ids: user
            .roles
            .iter()
            .filter_map(|r| r.organization_id)
            .collect(),
    }
}

#[derive(Debug, FromQueryResult)]
struct RelationshipRow {
    id: Id,
    organization_id: Id,
}

/// Resolve the relationship id set every searcher scopes on — once per request.
/// `None` = super admin with no narrowing filters (unrestricted). The
/// `organization_id` and `coaching_relationship_id` filters intersect here, so
/// they can never widen access, and searchers never see them.
async fn resolve_visible_relationship_ids(
    db: &DatabaseConnection,
    scope: &Scope,
    organization_id: Option<Id>,
    coaching_relationship_id: Option<Id>,
) -> Result<Option<Vec<Id>>, Error> {
    if scope.is_super_admin {
        return Ok(match (coaching_relationship_id, organization_id) {
            // Intersect-only: a bogus id yields an empty result, never an error.
            (Some(rel_id), _) => Some(vec![rel_id]),
            (None, Some(org_id)) => Some(
                coaching_relationships::Entity::find()
                    .select_only()
                    .column(coaching_relationships::Column::Id)
                    .column(coaching_relationships::Column::OrganizationId)
                    .filter(coaching_relationships::Column::OrganizationId.eq(org_id))
                    .into_model::<RelationshipRow>()
                    .all(db)
                    .await
                    .map_err(entity_api::error::Error::from)?
                    .into_iter()
                    .map(|r| r.id)
                    .collect(),
            ),
            (None, None) => None,
        });
    }

    let visible = coaching_relationships::Entity::find()
        .select_only()
        .column(coaching_relationships::Column::Id)
        .column(coaching_relationships::Column::OrganizationId)
        .filter(visible_to(scope))
        .into_model::<RelationshipRow>()
        .all(db)
        .await
        .map_err(entity_api::error::Error::from)?;

    Ok(Some(
        visible
            .into_iter()
            .filter(|r| organization_id.is_none_or(|org| r.organization_id == org))
            .map(|r| r.id)
            .filter(|id| coaching_relationship_id.is_none_or(|rel| *id == rel))
            .collect(),
    ))
}

/// The relationships where `user_id` is coach or coachee, intersected with the
/// caller's visible set — "by user" means participant for sessions.
async fn resolve_participant_relationship_ids(
    db: &DatabaseConnection,
    user_id: Id,
    visible: Option<&[Id]>,
) -> Result<Vec<Id>, Error> {
    let rows = coaching_relationships::Entity::find()
        .select_only()
        .column(coaching_relationships::Column::Id)
        .column(coaching_relationships::Column::OrganizationId)
        .filter(
            coaching_relationships::Column::CoachId
                .eq(user_id)
                .or(coaching_relationships::Column::CoacheeId.eq(user_id)),
        )
        .apply_if(visible, |q, ids| {
            q.filter(coaching_relationships::Column::Id.is_in(ids.iter().copied()))
        })
        .into_model::<RelationshipRow>()
        .all(db)
        .await
        .map_err(entity_api::error::Error::from)?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

/// Run a keyword search for the caller: resolve visibility once, fan out to the
/// selected searchers under bounded concurrency, merge by rank, paginate, and
/// hydrate the returned page.
pub async fn search(db: &DatabaseConnection, scope: &Scope, spec: Spec) -> Result<Results, Error> {
    let visible = resolve_visible_relationship_ids(
        db,
        scope,
        spec.filters.organization_id,
        spec.coaching_relationship_id,
    )
    .await?;

    // Nothing visible: an honestly empty page, no searcher round trips.
    if matches!(visible.as_deref(), Some([])) {
        return Ok(Results {
            query: spec.query,
            limit: spec.limit,
            hits: vec![],
            next_cursor: None,
        });
    }

    let mut filters = spec.filters;
    if let Some(user_id) = filters.user_id {
        filters.participant_relationship_ids =
            Some(resolve_participant_relationship_ids(db, user_id, visible.as_deref()).await?);
    }

    let request = Request {
        q: &spec.q_raw,
        scope,
        visible_relationship_ids: visible.as_deref(),
        filters: &filters,
        cursor: spec.cursor.as_ref(),
        // limit + 1 per searcher: the merged overflow row proves a next page
        // exists, and a shallower per-searcher fetch could strand rows behind
        // a cursor (see the pagination decision in the plan).
        fetch: u64::from(spec.limit) + 1,
    };

    let selected: Vec<_> = searchers()
        .into_iter()
        .filter(|s| spec.types.contains(&s.hit_type()))
        .collect();

    // Futures are created eagerly (they only run when polled), then drained
    // under the concurrency bound.
    let mut pending = Vec::with_capacity(selected.len());
    for searcher in &selected {
        pending.push(searcher.search(db, &request));
    }
    let results: Vec<Result<Vec<Hit>, entity_api::error::Error>> = stream::iter(pending)
        .buffer_unordered(SEARCHER_CONCURRENCY)
        .collect()
        .await;

    let mut merged: Vec<Hit> = results
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();

    merged.sort_by(|a, b| {
        b.core()
            .score
            .total_cmp(&a.core().score)
            .then_with(|| a.hit_type().cmp(&b.hit_type()))
            .then_with(|| a.core().id.cmp(&b.core().id))
    });

    let limit = usize::from(spec.limit);
    let next_cursor = (merged.len() > limit).then(|| {
        let last = &merged[limit - 1];
        Cursor {
            score: last.core().score,
            hit_type: last.hit_type(),
            id: last.core().id,
        }
        .encode()
    });
    merged.truncate(limit);

    hydrate_session_display_titles(db, &mut merged).await?;
    hydrate_snippets(db, &spec.q_raw, &mut merged).await?;

    Ok(Results {
        query: spec.query,
        limit: spec.limit,
        hits: merged,
        next_cursor,
    })
}

/// Fill session display titles for the returned page: a `SessionHit`'s
/// `title`/`display_title` and the `session_display_title` context on
/// action/agreement hits. The composed title falls back to
/// "Coaching session — YYYY-MM-DD" (naive session date, identical for every
/// caller), so `title` is never empty by construction.
async fn hydrate_session_display_titles(
    db: &DatabaseConnection,
    hits: &mut [Hit],
) -> Result<(), Error> {
    let session_ids: Vec<Id> = hits
        .iter()
        .filter_map(|h| match h {
            Hit::CoachingSession(s) => Some(s.core.id),
            Hit::Action(a) => Some(a.coaching_session_id),
            Hit::Agreement(a) => Some(a.coaching_session_id),
            _ => None,
        })
        .collect();
    if session_ids.is_empty() {
        return Ok(());
    }

    let sessions = coaching_sessions::Entity::find()
        .filter(coaching_sessions::Column::Id.is_in(session_ids))
        .all(db)
        .await
        .map_err(entity_api::error::Error::from)?;
    let composed = batch_load_display_titles(db, &sessions).await?;

    let titles: std::collections::HashMap<Id, String> = sessions
        .iter()
        .map(|s| {
            let title = composed
                .get(&s.id)
                .cloned()
                .flatten()
                .unwrap_or_else(|| format!("Coaching session — {}", s.date.format("%Y-%m-%d")));
            (s.id, title)
        })
        .collect();

    for hit in hits.iter_mut() {
        match hit {
            Hit::CoachingSession(s) => {
                if let Some(title) = titles.get(&s.core.id) {
                    s.core.title = title.clone();
                    s.display_title = title.clone();
                }
            }
            Hit::Action(a) => {
                if let Some(title) = titles.get(&a.coaching_session_id) {
                    a.session_display_title = title.clone();
                }
            }
            Hit::Agreement(a) => {
                if let Some(title) = titles.get(&a.coaching_session_id) {
                    a.session_display_title = title.clone();
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
