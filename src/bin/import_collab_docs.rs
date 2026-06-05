//! Importer binary: copies TipTap Cloud documents into `collab_documents`.
//!
//! Default run upserts; pass `--dry-run` to classify and export without writing.

use log::{error, info};
use service::{config::Config, logging::Logger};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    service::load_env_file();
    let config = Config::new();
    Logger::init_logger(&config as &Config);

    let dry_run = std::env::args().any(|a| a == "--dry-run");
    info!("Importing TipTap Cloud documents into collab_documents (dry_run={dry_run})...");

    let db = match service::init_database(&config).await {
        Ok(db) => Arc::new(db),
        Err(e) => {
            error!("Failed to establish database connection: {e}");
            std::process::exit(1);
        }
    };

    match domain::collab_import::import_cloud_documents(&config, db.as_ref(), dry_run).await {
        Ok(summary) => {
            info!(
                "Import finished: found={} would_write={} written={} \
                 skipped_no_session={} skipped_archived={} skipped_empty={} failed={}",
                summary.found,
                summary.would_write,
                summary.written,
                summary.skipped_no_session,
                summary.skipped_archived,
                summary.skipped_empty,
                summary.failed,
            );
        }
        Err(e) => {
            error!("Import failed: {e}");
            std::process::exit(1);
        }
    }
}
