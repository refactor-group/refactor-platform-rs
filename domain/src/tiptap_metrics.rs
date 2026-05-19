//! TipTap platform-level metrics (aggregates across all organizations).
//!
//! Domain wrapper over `gateway::tiptap_metrics`. Composes raw gateway
//! responses into the shapes admin endpoints actually surface

use std::collections::{HashMap, HashSet};

use sea_orm::DatabaseConnection;
use serde::Serialize;
use service::config::Config;

use crate::error::Error;
use crate::gateway::tiptap_metrics::{Client, Document};
use crate::Id;

/// Soft cap on returned abandoned docs. A wall of thousands buries useful
/// signal; admins get a representative sample plus the true total.
const ABANDONED_LIMIT: usize = 500;

/// A TipTap document with no matching coaching_session.
///
/// `archived` is surfaced because the archived-but-still-billed docs are the
/// most common kind of leak - admins should see them distince from live ones.
#[derive(Debug, Clone, Serialize)]
pub struct AbandonedDoc {
    pub document_name: String,
    pub size_bytes: u64,
    pub archived: bool,
}

/// Result of an abandoned-docs reconciliation pass.
///
/// `total_found` reports the *true* count even when `abandoned` is capped
/// `truncated` is the explicit flag UIs need to render "Showing N of M".
#[derive(Debug, Clone, Serialize)]
pub struct AbandonedReport {
    pub abandoned: Vec<AbandonedDoc>,
    pub total_found: u64,
    pub truncated: bool,
}

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

/// Find TipTap documents whose parent coaching_session no longer exists.
/// Snapshot order: TipTap first, DB second. This minimizes false negatives
/// on session deletes (the case admins care about). The cost is a small
/// false-positive window for in-flight session creations - interleaving
/// HTTP and DB writes means a session whose TipTap doc landed before its
/// DB row may transiently appear here. Admins should verify before any
/// destructive cleanup; this is an observability signal, not an automation.
pub async fn abandoned_documents(
    db: &DatabaseConnection,
    config: &Config,
) -> Result<AbandonedReport, Error> {
    let client = Client::new(config).await?;

    // Snapshot TipTap first - it's the slower source (HTTP, paginated).
    // Reading it first means any session created during the walk that
    // *also* finished its TipTap write before TO will be captured below
    // when we read the DB
    let tiptap_docs = client.list_all_documents().await?;

    // Snapshot DB second.
    let session_names = entity_api::tiptap_metrics::all_collab_document_names(db).await?;

    Ok(reconcile_abandoned(&tiptap_docs, &session_names))
}

/// Pure set-diff: TipTap docs MINUS DB session names = abandoned.
///
/// Extracted from the orchestrator so the algorithm is unit-testable
/// without HTTP or DB
fn reconcile_abandoned(docs: &[Document], session_names: &[String]) -> AbandonedReport {
    // `&str` keys borrow from session_names - no cloning. 0(1) lookup
    let session_set: HashSet<&str> = session_names.iter().map(String::as_str).collect();

    let mut abandoned: Vec<AbandonedDoc> = docs
        .iter()
        .filter(|d| !session_set.contains(d.name.as_str()))
        .map(|d| AbandonedDoc {
            document_name: d.name.clone(),
            size_bytes: d.size,
            archived: d.archived,
        })
        .collect();

    // Compute the true Total BEFORE truncating - admins need accurate scale.
    let total_found = abandoned.len() as u64;
    let truncated = abandoned.len() > ABANDONED_LIMIT;

    // Sort biggest first, THEN truncate. Truncate-then-sort would give an
    // arbitrary slice; this way the most impactful leaks are always visible.
    abandoned.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    abandoned.truncate(ABANDONED_LIMIT);

    AbandonedReport {
        abandoned,
        total_found,
        truncated,
    }
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

    #[test]
    fn reconcile_abandoned_returns_tiptap_docs_with_no_matching_session() {
        use crate::gateway::tiptap_metrics::Document;

        let docs = vec![
            // Mapped to a live session - not abandoned
            Document {
                name: "live-a".to_string(),
                size: 100,
                archived: false,
            },
            // No matching session - abandoned, big.
            Document {
                name: "ghost-big".to_string(),
                size: 9_999,
                archived: false,
            },
            // Mapped - not abandoned.
            Document {
                name: "live-b".to_string(),
                size: 50,
                archived: false,
            },
            // Archived AND orphaned - still abandoned, with archived: true.
            Document {
                name: "ghost-archived".to_string(),
                size: 1_000,
                archived: true,
            },
            // No matching session - abandoned, smallest.
            Document {
                name: "ghost-small".to_string(),
                size: 5,
                archived: false,
            },
        ];

        let session_names = vec!["live-a".to_string(), "live-b".to_string()];
        let report = reconcile_abandoned(&docs, &session_names);

        assert_eq!(report.total_found, 3);
        assert!(!report.truncated);
        assert_eq!(report.abandoned.len(), 3);

        // Sort by size desc: ghost-big (9999) > ghost-archived (1000) > ghost-small (5).
        assert_eq!(report.abandoned[0].document_name, "ghost-big");
        assert_eq!(report.abandoned[0].size_bytes, 9_999);
        assert_eq!(report.abandoned[1].document_name, "ghost-archived");
        assert!(report.abandoned[1].archived);
        assert_eq!(report.abandoned[2].document_name, "ghost-small");
    }

    #[test]
    fn reconcile_abandoned_truncates_and_reports_total() {
        use crate::gateway::tiptap_metrics::Document;

        // 502 orphand -> truncated to 500, total found = 502
        let docs: Vec<Document> = (0..502)
            .map(|i| Document {
                name: format!("ghost-{i:04}"),
                // Make sizes distinct so sort order is well-defined.
                size: (502 - i) as u64,
                archived: false,
            })
            .collect();

        let report = reconcile_abandoned(&docs, &[]);

        assert_eq!(report.total_found, 502);
        assert!(report.truncated);
        assert_eq!(report.abandoned.len(), 500);
        // Biggest survived the truncate.
        assert_eq!(report.abandoned[0].size_bytes, 502);
    }
}
