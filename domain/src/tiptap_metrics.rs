//! TipTap platform-level metrics (aggregates across all organizations).
//!
//! Domain wrapper over `gateway::tiptap_metrics`. Composes raw gateway
//! responses into the shapes admin endpoints actually surface

use serde::Serialize;
use service::config::Config;

use crate::error::Error;
use crate::gateway::tiptap_metrics::Client;

/// Aggregate TipTap-document mentrics across all orgs
///
/// Bytes are stored raw - formatting to MB/GB happens in the display layer
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
    let totals = docs
        .iter()
        .fold(PlatformTotals::default(), |mut acc, doc| {
            
        })
}