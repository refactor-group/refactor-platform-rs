//! One-per-request visibility resolution: the relationship id sets every
//! searcher scopes on. Query construction lives here (entity_api layer); the
//! domain orchestrator decides when to call these and what to do with an
//! empty result.

use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, QueryFilter, QuerySelect, QueryTrait,
};

use crate::error::Error;
use entity::coaching_relationships::{self, visible_to, Visibility};
use entity::Id;

/// Resolve the relationship id set every searcher scopes on — once per request.
/// `None` = super admin with no narrowing filters (unrestricted). The
/// `organization_id` and `coaching_relationship_id` filters intersect here, so
/// they can never widen access, and searchers never see them.
pub async fn resolve_visible_relationship_ids(
    db: &DatabaseConnection,
    visibility: &Visibility,
    organization_id: Option<Id>,
    coaching_relationship_id: Option<Id>,
) -> Result<Option<Vec<Id>>, Error> {
    // A super admin without an org filter needs no query: unrestricted, or the
    // one requested relationship (a bogus id yields an empty result, never an error).
    if visibility.is_super_admin && organization_id.is_none() {
        return Ok(coaching_relationship_id.map(|rel_id| vec![rel_id]));
    }

    // Super admins skip `visible_to`; otherwise the two paths are one query.
    Ok(Some(
        coaching_relationships::Entity::find()
            .select_only()
            .column(coaching_relationships::Column::Id)
            .apply_if(
                (!visibility.is_super_admin).then(|| visible_to(visibility)),
                |q, condition| q.filter(condition),
            )
            .apply_if(organization_id, |q, org| {
                q.filter(coaching_relationships::Column::OrganizationId.eq(org))
            })
            .apply_if(coaching_relationship_id, |q, rel| {
                q.filter(coaching_relationships::Column::Id.eq(rel))
            })
            .into_tuple::<Id>()
            .all(db)
            .await?,
    ))
}

/// The relationships where `user_id` is coach or coachee, intersected with the
/// caller's visible set — "by user" means participant for sessions.
pub async fn resolve_participant_relationship_ids(
    db: &DatabaseConnection,
    user_id: Id,
    visible: Option<&[Id]>,
) -> Result<Vec<Id>, Error> {
    Ok(coaching_relationships::Entity::find()
        .select_only()
        .column(coaching_relationships::Column::Id)
        .filter(
            coaching_relationships::Column::CoachId
                .eq(user_id)
                .or(coaching_relationships::Column::CoacheeId.eq(user_id)),
        )
        .apply_if(visible, |q, ids| {
            q.filter(coaching_relationships::Column::Id.is_in(ids.iter().copied()))
        })
        .into_tuple::<Id>()
        .all(db)
        .await?)
}

#[cfg(all(test, feature = "mock"))]
#[path = "visibility_mock_tests.rs"]
mod mock_tests;
