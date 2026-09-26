//! One-per-request visibility resolution: the relationship id sets every
//! searcher scopes on. Query construction lives here (entity_api layer); the
//! domain orchestrator decides when to call these and what to do with an
//! empty result.

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, FromQueryResult, QueryFilter,
    QuerySelect, QueryTrait,
};

use crate::error::Error;
use entity::coaching_relationships::{self, visible_to, Scope};
use entity::Id;

#[derive(Debug, FromQueryResult)]
struct RelationshipRow {
    id: Id,
    organization_id: Id,
}

/// Resolve the relationship id set every searcher scopes on — once per request.
/// `None` = super admin with no narrowing filters (unrestricted). The
/// `organization_id` and `coaching_relationship_id` filters intersect here, so
/// they can never widen access, and searchers never see them.
pub async fn resolve_visible_relationship_ids(
    db: &DatabaseConnection,
    scope: &Scope,
    organization_id: Option<Id>,
    coaching_relationship_id: Option<Id>,
) -> Result<Option<Vec<Id>>, Error> {
    if scope.is_super_admin {
        return Ok(match (coaching_relationship_id, organization_id) {
            // Intersect-only: a bogus id yields an empty result, never an error.
            (Some(rel_id), None) => Some(vec![rel_id]),
            (None, None) => None,
            // Any org filter hits the DB once; a present relationship filter
            // intersects — the relationship is kept only if it belongs to the
            // requested organization.
            (rel_id, Some(org_id)) => Some(
                coaching_relationships::Entity::find()
                    .select_only()
                    .column(coaching_relationships::Column::Id)
                    .column(coaching_relationships::Column::OrganizationId)
                    .filter(coaching_relationships::Column::OrganizationId.eq(org_id))
                    .apply_if(rel_id, |q, rel| {
                        q.filter(coaching_relationships::Column::Id.eq(rel))
                    })
                    .into_model::<RelationshipRow>()
                    .all(db)
                    .await?
                    .into_iter()
                    .map(|r| r.id)
                    .collect(),
            ),
        });
    }

    let visible = coaching_relationships::Entity::find()
        .select_only()
        .column(coaching_relationships::Column::Id)
        .column(coaching_relationships::Column::OrganizationId)
        .filter(visible_to(scope))
        .into_model::<RelationshipRow>()
        .all(db)
        .await?;

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
pub async fn resolve_participant_relationship_ids(
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
        .await?;
    Ok(rows.into_iter().map(|r| r.id).collect())
}

#[cfg(all(test, feature = "mock"))]
#[path = "visibility_mock_tests.rs"]
mod mock_tests;
