//! TipTap platform-level metrics (aggregates across all organizations).
//!
//! Domain wrapper over `gateway::tiptap_metrics`. Composes raw gateway
//! responses into the shapes admin endpoints actually surface

use std::collections::HashMap;

use sea_orm::DatabaseConnection;
use serde::Serialize;
use service::config::Config;

use crate::error::Error;
use crate::gateway::tiptap_metrics::{Client, Document};
use crate::Id;

/// Aggregate TipTap-document metrics across all orgs.
///
/// Bytes are stored raw - formatting to MB/GB happens in the display layer.
/// "Compute once, format many" keeps the API stable as units evolve.
// `Debug` for logs, `Clone` cheap, `Default` lets us seed the fold accumulator,
// `Serialize` because this gets JSON-encoded by the web layer.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PlatformTotals {
    /// Live (non-archived) document count.
    pub total_documents: u64,
    /// Total bytes across live documents.
    pub total_bytes: u64,
    /// Count of documents flagged archived in TipTap.
    pub archived_documents: u64,
}

/// Walk every TipTap document and roll up platform-wide totals.
///
/// No DB hits - pure gateway -> aggregation. Domain composes the gateway;
/// web composes the domain.
pub async fn platform_totals(config: &Config) -> Result<PlatformTotals, Error> {
    // Build a client per call. `reqwest::Client` is cheap to construct; the
    // expensive part (the connection pool) warms on first request. Hoist into
    // AppState only if profiling shows setup is a hotspot
    let client = Client::new(config).await?;
    let docs = client.list_all_documents().await?;

    // `iter().fold` is the idiomatic multi-counter accumulator. Equivalent to
    // a `for` loop with several `let mut`s, but read as "transform input
    // into output" - easier to reason about
    let totals = docs.iter().fold(PlatformTotals::default(), |mut acc, doc| {
        if doc.archived {
            acc.archived_documents += 1;
        } else {
            acc.total_documents += 1;
            // `size` is `u64`; sums won't overflow at realistic scales.
            // If we ever cross 18 exabytes total we have bigger problems.
            acc.total_bytes += doc.size;
        }
        acc
    });
    Ok(totals)
}

// Re-export the row type for tests (and any future caller in this module).
pub use entity_api::tiptap_metrics::SessionOrgRow;

/// Per-organization TipTap document metrics.
///
/// One row per org that owns at least one live (non-archived) document.
/// Org name is included so admin UIs don't need a follow-up lookup.
#[derive(Debug, Clone, Serialize)]
pub struct OrgMetrics {
    pub organization_id: Id,
    pub organization_name: String,
    pub document_count: u64,
    pub total_bytes: u64,
}

/// Aggregate TipTap documents by owning organization.
///
/// Strategy:
/// 1. Walk TipTap -> live docs.
/// 2. Bulk-fetch (doc_name -> org_id + name) via one 3-way join.
/// 3. Aggregate in app code (TipTap = size, DB = org assignment; only Rust
///    can combine them).
///
/// Docs with no matching coaching_session are silently skipped - they're
/// "abandoned" and surfaced separately in Session 7.
pub async fn per_org_metrics(
    db: &DatabaseConnection,
    config: &Config,
) -> Result<Vec<OrgMetrics>, Error> {
    let client = Client::new(config).await?;
    let docs = client.list_all_documents().await?;

    // Collect live doc names for the DB lookup. Archived docs don't contribute
    // to per-org totals (matches `platform_totals` semantics).
    let doc_names: Vec<String> = docs
        .iter()
        .filter(|d| !d.archived)
        .map(|d| d.name.clone())
        .collect();
    if doc_names.is_empty() {
        return Ok(Vec::new());
    }

    // One bulk query - no N+1. `?` propagates entity_api::Error -> domain::Error
    // via the existing From impl.
    let rows =
        entity_api::tiptap_metrics::find_sessions_with_org_by_doc_names(db, doc_names).await?;

    Ok(aggregate_by_org(&docs, &rows))
}

/// Pure aggregation: combine TipTap sizes with DB org assignments.
///
/// Extracted so it's unit-testable without MockDatabase. The orchestration
/// (HTTP + DB) is integration-tested in Session 9; this is the algorithm.
fn aggregate_by_org(docs: &[Document], rows: &[SessionOrgRow]) -> Vec<OrgMetrics> {
    // Pre-index live doc sizes by name -> O(1) lookup during the row walk.
    // `&str` keys avoid cloning every doc name into the HashMap.
    let size_by_name: HashMap<&str, u64> = docs
        .iter()
        .filter(|d| !d.archived)
        .map(|d| (d.name.as_str(), d.size))
        .collect();

    // Per-org accumulator. `entry().or_insert_with(...)` is the idiomatic
    // group-by pattern - single hash lookup per insert vs contains_key+insert.
    let mut by_org: HashMap<Id, OrgMetrics> = HashMap::new();
    for row in rows {
        // let-else: bind the inner String if present, otherwise skip this row.
        // Cleaner than nested `if let` for "filter + continue" loops.
        let Some(doc_name) = &row.collab_document_name else {
            continue;
        };
        let Some(&size) = size_by_name.get(doc_name.as_str()) else {
            continue;
        };

        let entry = by_org
            .entry(row.organization_id)
            .or_insert_with(|| OrgMetrics {
                organization_id: row.organization_id,
                organization_name: row.organization_name.clone(),
                document_count: 0,
                total_bytes: 0,
            });
        entry.document_count += 1;
        entry.total_bytes += size;
    }

    // HashMap iteration order is non-deterministic. Sort by count desc so
    // admins see biggest tenants first AND test assertions stay stable.
    let mut result: Vec<OrgMetrics> = by_org.into_values().collect();
    result.sort_by(|a, b| b.document_count.cmp(&a.document_count));
    result
}

// ------------------------------------------------
// Tests
// ------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;
    use serde_json::json;

    fn test_config(tiptap_url: &str) -> Config {
        Config::from_args([
            "test",
            "--tiptap-auth-key=test-auth-key",
            &format!("--tiptap-url={tiptap_url}"),
        ])
    }

    /// Three documents: two live (sizes 1000, 500) + one archived. Asserts
    /// the fold's branch logic counts and sums correctly, and that archived
    /// docs are excluded from `total_bytes` even though their size is non-zero.
    #[tokio::test]
    async fn platform_totals_aggregates_live_and_archived() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let body = json!([
            { "name": "doc-a", "size": 1000, "archived": false },
            { "name": "doc-b", "size": 500, "archived": false },
            // Archived size deliberately non-zero - verifies we don't add it.
            { "name": "doc-c", "size": 9999, "archived": true },
        ])
        .to_string();

        // Short page (3 < PAGE_SIZE=100) -> loop terminates after one call.
        let _mock = server
            .mock("GET", "/api/documents")
            .match_query(mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let config = test_config(&server.url());
        let totals = platform_totals(&config).await?;

        assert_eq!(totals.total_documents, 2);
        assert_eq!(totals.total_bytes, 1500);
        assert_eq!(totals.archived_documents, 1);
        Ok(())
    }

    /// Unit test for the pure aggregator. Uses hand-built fixtures so we
    /// don't need MockDatabase. Covers: per-org grouping, byte sum, archived
    /// exclusion, unmapped-doc skip, and sort-by-count-desc ordering.
    #[test]
    fn aggregate_by_org_groups_sums_and_skips_correctly() {
        let org_x = Id::new_v4();
        let org_y = Id::new_v4();

        let docs = vec![
            Document {
                name: "a".to_string(),
                size: 100,
                archived: false,
            },
            Document {
                name: "b".to_string(),
                size: 200,
                archived: false,
            },
            Document {
                name: "c".to_string(),
                size: 50,
                archived: false,
            },
            // Archived - should not contribute even though "d" is mapped below.
            Document {
                name: "d".to_string(),
                size: 999,
                archived: true,
            },
            // Unmapped (no row matches) - silently dropped (abandoned).
            Document {
                name: "ghost".to_string(),
                size: 7,
                archived: false,
            },
        ];

        let rows = vec![
            SessionOrgRow {
                collab_document_name: Some("a".to_string()),
                organization_id: org_x,
                organization_name: "X".to_string(),
            },
            SessionOrgRow {
                collab_document_name: Some("b".to_string()),
                organization_id: org_x,
                organization_name: "X".to_string(),
            },
            SessionOrgRow {
                collab_document_name: Some("c".to_string()),
                organization_id: org_y,
                organization_name: "Y".to_string(),
            },
        ];

        let result = aggregate_by_org(&docs, &rows);
        // Two orgs, X first (2 docs > 1 doc).
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].organization_id, org_x);
        assert_eq!(result[0].document_count, 2);
        assert_eq!(result[0].total_bytes, 300);
        assert_eq!(result[1].organization_id, org_y);
        assert_eq!(result[1].document_count, 1);
        assert_eq!(result[1].total_bytes, 50);
    }
}
