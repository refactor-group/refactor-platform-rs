//! TipTap Cloud -> `collab_documents` importer.
//!
//! Lists Cloud documents, exports each as a raw Yjs v1 binary update, and
//! upserts it into `refactor_platform.collab_documents` (in whatever database
//! `db` connects to, i.e. the collab DB) keyed by name. Every non-archived,
//! non-empty Cloud document is copied; there is no coaching-session
//! intersection, so `collab_documents` may live in its own database, separate
//! from `coaching_sessions`. Docs whose sessions were since deleted import as
//! harmless orphan rows (nothing references them, so they are never served).
//! Has a dry-run mode that classifies and exports but writes nothing.

use log::{info, warn};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use service::config::Config;

use crate::error::Error;
use crate::gateway::tiptap_metrics::{Client, Document};

/// Tally of one import pass. `written` stays 0 in dry-run; `would_write` is the
/// dry-run projection of how many docs a real run would upsert.
#[derive(Debug, Clone, Default)]
pub struct ImportSummary {
    /// Total docs Cloud listed.
    pub found: usize,
    /// Docs upserted into collab_documents (0 in dry-run).
    pub written: usize,
    /// Eligible docs that exported OK (the dry-run count).
    pub would_write: usize,
    /// Skipped: archived in Cloud.
    pub skipped_archived: usize,
    /// Skipped: zero-size (no content).
    pub skipped_empty: usize,
    /// Listed but export failed or vanished.
    pub failed: usize,
}

/// One listed Cloud document, surfaced by `--list` so an operator can preview
/// the inventory before an import. Mirrors the gateway's internal `Document`
/// without leaking it across the crate boundary.
#[derive(Debug, Clone)]
pub struct CloudDocInfo {
    pub name: String,
    pub size: u64,
    pub archived: bool,
}

/// List every TipTap Cloud document (read-only; no DB access). Backs the
/// importer's `--list` discovery mode.
pub async fn list_cloud_documents(config: &Config) -> Result<Vec<CloudDocInfo>, Error> {
    let client = Client::new(config)?;
    Ok(client
        .list_all_documents()
        .await?
        .into_iter()
        .map(|d| CloudDocInfo {
            name: d.name,
            size: d.size,
            archived: d.archived,
        })
        .collect())
}

/// Eligibility classes for a listed Cloud document.
enum Class {
    Eligible,
    Archived,
    Empty,
}

/// Pure eligibility check from the Cloud list metadata alone: archived first,
/// then empty, otherwise eligible.
fn classify(doc: &Document) -> Class {
    if doc.archived {
        Class::Archived
    } else if doc.size == 0 {
        Class::Empty
    } else {
        Class::Eligible
    }
}

/// Idempotent upsert of one document's Yjs state by name.
async fn upsert_document(db: &DatabaseConnection, name: &str, state: Vec<u8>) -> Result<(), Error> {
    let stmt = Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO refactor_platform.collab_documents (name, state) VALUES ($1, $2) \
         ON CONFLICT (name) DO UPDATE SET state = EXCLUDED.state, updated_at = now()",
        [name.into(), state.into()],
    );
    db.execute(stmt)
        .await
        .map_err(entity_api::error::Error::from)?;
    Ok(())
}

/// Copy every non-archived, non-empty TipTap Cloud document into
/// `collab_documents`.
///
/// `dry_run` classifies and exports but writes nothing, so the returned
/// `would_write` previews a real run without mutating the table.
pub async fn import_cloud_documents(
    config: &Config,
    db: &DatabaseConnection,
    dry_run: bool,
) -> Result<ImportSummary, Error> {
    let client = Client::new(config)?;
    let docs = client.list_all_documents().await?;

    let mut summary = ImportSummary {
        found: docs.len(),
        ..Default::default()
    };

    for doc in &docs {
        match classify(doc) {
            Class::Archived => summary.skipped_archived += 1,
            Class::Empty => summary.skipped_empty += 1,
            Class::Eligible => match client.export_document(&doc.name).await {
                Ok(Some(bytes)) => {
                    summary.would_write += 1;
                    if !dry_run {
                        upsert_document(db, &doc.name, bytes).await?;
                        summary.written += 1;
                    }
                }
                // Listed by the index but gone at export time.
                Ok(None) => summary.failed += 1,
                Err(e) => {
                    warn!("Export failed for document {}: {e}", doc.name);
                    summary.failed += 1;
                }
            },
        }
    }

    info!(
        "Cloud import complete (dry_run={dry_run}): found={} would_write={} written={} \
         skipped_archived={} skipped_empty={} failed={}",
        summary.found,
        summary.would_write,
        summary.written,
        summary.skipped_archived,
        summary.skipped_empty,
        summary.failed,
    );

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(name: &str, size: u64, archived: bool) -> Document {
        Document {
            name: name.to_string(),
            size,
            archived,
        }
    }

    /// classify order: archived beats empty beats eligible.
    #[test]
    fn classify_applies_checks_in_documented_order() {
        // Archived checked before empty.
        assert!(matches!(classify(&doc("d", 0, true)), Class::Archived));
        // Not archived + size 0 -> Empty.
        assert!(matches!(classify(&doc("d", 0, false)), Class::Empty));
        // Not archived + size > 0 -> Eligible.
        assert!(matches!(classify(&doc("d", 10, false)), Class::Eligible));
    }
}
