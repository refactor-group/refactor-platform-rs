//! TipTap document metrics queries.
//! Joins coaching_sessions -> coaching_relationships -> organizations to map
//! TipTap document identifiers (= collab_document_name) back to their
//! owning organization. Domain layer aggregates the rows; this module just
//! fetches them

use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, JoinType, QueryFilter, QuerySelect,
    RelationTrait,
};

use crate::error::Error;
use crate::{coaching_relationships, coaching_sessions, organizations, Id};

/// One row per coaching_session whose collab_document_name matches a
/// requested doc name. Built via SeaORM's `FromQueryResult` so a multi-table
/// projection materializes into a typed struct.
///
/// `collab_document_name` is `Option<String>` to match the source column -
/// nullable in the schema even though `is_in(...)` excludes NULLs. Matching
/// the source type avoids deserialization panics on edge cases
#[derive(FromQueryResult, Debug, Clone, PartialEq, Eq)]
pub struct SessionOrgRow {
    pub collab_document_name: Option<String>,
    pub organization_id: Id,
    pub organization_name: String,
}

/// Fetch session -> org mappings for a batch of TipTap document names.
/// Single round-trip via a 3-way inner join. No N+1: one query per call
/// regardless of how many doc names are in scope.
pub async fn find_sessions_with_org_by_doc_names(
    db: &impl ConnectionTrait,
    doc_names: Vec<String>,
) -> Result<Vec<SessionOrgRow>, Error> {
    // Defensive: empty IN clause is a SQL footgun. Short-circuit instead.
    if doc_names.is_empty() {
        return Ok(Vec::new());
    }

    Ok(coaching_sessions::Entity::find()
        // Two chained inner joins: sessions -> relationships -> organizations.
        // SeaORM tracks the "current" table for each subsequent join, so the
        // second `Relation::Organizations` resolves against coaching_relationships.
        .join(
            JoinType::InnerJoin,
            coaching_sessions::Relation::CoachingRelationships.def(),
        )
        .join(
            JoinType::InnerJoin,
            coaching_relationships::Relation::Organizations.def(),
        )
        // `is_in` translates to SQL `IN (...)`. NULLs are auto-excluded by IN.
        .filter(coaching_sessions::Column::CollabDocumentName.is_in(doc_names))
        // `select_only` switches off the default "select all columns of E".
        // From here, only explicitly-added columns appear in the projection.
        .select_only()
        .column(coaching_sessions::Column::CollabDocumentName)
        // `column_as` lets us alias columns to match the FromQueryResult
        // struct field names. Required when columns from different tables
        // share names (here, `id` and `name`).
        .column_as(organizations::Column::Id, "organization_id")
        .column_as(organizations::Column::Name, "organization_name")
        // Materialize each row into the typed struct.
        .into_model::<SessionOrgRow>()
        .all(db)
        .await?)
}
