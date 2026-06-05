//! TipTap Cloud -> `collab_documents` importer.
//!
//! Lists Cloud documents, exports each as a raw Yjs v1 binary update, and
//! upserts it into `refactor_platform.collab_documents` keyed by name. Only
//! documents that map to a coaching session and carry live content are copied.
//! Has a dry-run mode that classifies and exports but writes nothing.

use std::collections::HashSet;

use log::{info, warn};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use service::config::Config;

use entity_api::tiptap_metrics::all_collab_document_names;

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
    /// Skipped: no matching coaching session.
    pub skipped_no_session: usize,
    /// Skipped: archived in Cloud.
    pub skipped_archived: usize,
    /// Skipped: zero-size (no content).
    pub skipped_empty: usize,
    /// Listed but export failed or vanished.
    pub failed: usize,
}

/// Eligibility classes for a listed Cloud document.
enum Class {
    Eligible,
    NoSession,
    Archived,
    Empty,
}

/// Pure eligibility check. Order matters: no-session, then archived, then empty.
/// A doc with no matching session is NoSession even when archived.
fn classify(doc: &Document, names: &HashSet<String>) -> Class {
    if !names.contains(&doc.name) {
        Class::NoSession
    } else if doc.archived {
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

/// Copy eligible TipTap Cloud documents into `collab_documents`.
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

    let names: HashSet<String> = all_collab_document_names(db).await?.into_iter().collect();

    let mut summary = ImportSummary {
        found: docs.len(),
        ..Default::default()
    };

    for doc in &docs {
        match classify(doc, &names) {
            Class::NoSession => summary.skipped_no_session += 1,
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
         skipped_no_session={} skipped_archived={} skipped_empty={} failed={}",
        summary.found,
        summary.would_write,
        summary.written,
        summary.skipped_no_session,
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

    /// classify order: no-session beats archived beats empty.
    #[test]
    fn classify_applies_checks_in_documented_order() {
        let names: HashSet<String> = ["present".to_string()].into_iter().collect();

        // Absent name -> NoSession even when archived.
        assert!(matches!(
            classify(&doc("absent", 0, true), &names),
            Class::NoSession
        ));
        // Present + archived -> Archived (archived checked before empty).
        assert!(matches!(
            classify(&doc("present", 0, true), &names),
            Class::Archived
        ));
        // Present + size 0 -> Empty.
        assert!(matches!(
            classify(&doc("present", 0, false), &names),
            Class::Empty
        ));
        // Present + size > 0 -> Eligible.
        assert!(matches!(
            classify(&doc("present", 10, false), &names),
            Class::Eligible
        ));
    }
}
