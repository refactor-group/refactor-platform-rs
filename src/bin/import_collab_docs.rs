//! Importer binary: copies TipTap Cloud documents into `collab_documents`.
//!
//! Modes are selected by environment variable, not CLI args: `Config::new()`
//! parses argv strictly via clap and would reject unknown flags. `IMPORT_LIST=1`
//! lists Cloud documents (read-only, no DB); `IMPORT_DRY_RUN=1` classifies and
//! exports without writing; unset upserts for real.

use log::{error, info};
use service::{config::Config, logging::Logger};
use std::sync::Arc;

/// Read a boolean importer-mode flag from the environment. True for "1" or
/// "true" (case-insensitive); absent or anything else is false.
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

#[tokio::main]
async fn main() {
    service::load_env_file();
    let config = Config::new();
    Logger::init_logger(&config as &Config);

    let list_mode = env_flag("IMPORT_LIST");
    let dry_run = env_flag("IMPORT_DRY_RUN");

    // IMPORT_LIST is a read-only Cloud discovery mode: no DB connection, no writes.
    if list_mode {
        match domain::collab_import::list_cloud_documents(&config).await {
            Ok(docs) => {
                info!("Cloud documents listed: {}", docs.len());
                for d in &docs {
                    info!("  {} (size={}, archived={})", d.name, d.size, d.archived);
                }
            }
            Err(e) => {
                error!("List failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

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
