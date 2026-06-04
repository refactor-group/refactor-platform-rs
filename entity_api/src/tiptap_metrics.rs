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

/// Max doc names per `IN (...)` batch. PostgreSQL caps a single statement at
/// 65,535 bind parameters; batching keeps a large doc set under that ceiling.
const DOC_NAME_BATCH_SIZE: usize = 10_000;

/// Fetch session -> org mappings for a set of TipTap document names.
/// A 3-way inner join per batch; doc names are chunked so a very large set
/// can't exceed PostgreSQL's bind-parameter limit. No N+1 within a batch.
pub async fn find_sessions_with_org_by_doc_names(
    db: &impl ConnectionTrait,
    doc_names: Vec<String>,
) -> Result<Vec<SessionOrgRow>, Error> {
    // Defensive: empty IN clause is a SQL footgun. Short-circuit instead.
    if doc_names.is_empty() {
        return Ok(Vec::new());
    }

    let mut rows = Vec::new();
    for batch in doc_names.chunks(DOC_NAME_BATCH_SIZE) {
        let batch_rows = coaching_sessions::Entity::find()
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
            .filter(coaching_sessions::Column::CollabDocumentName.is_in(batch.iter().cloned()))
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
            .await?;
        rows.extend(batch_rows);
    }
    Ok(rows)
}

/// Dump every non-null collab_document_name from coaching sessions.
///
/// Used by domain::tiptap_metrics::abandoned_documents to set diff against
/// the TipTap document list. Single-column projection - no entity hydration.
pub async fn all_collab_document_names(db: &impl ConnectionTrait) -> Result<Vec<String>, Error> {
    // `into_tuple::<Option<String>>()` returns one Option per row for a
    // single-column select. `flatten()` on Vec<Option<T>> drops the Nones
    // and unwraps the Somes - defensive even though `is_not_null` should
    // prevent NULLs from reaching us.
    Ok(coaching_sessions::Entity::find()
        .filter(coaching_sessions::Column::CollabDocumentName.is_not_null())
        .select_only()
        .column(coaching_sessions::Column::CollabDocumentName)
        .into_tuple::<Option<String>>()
        .all(db)
        .await?
        .into_iter()
        .flatten()
        .collect())
}
