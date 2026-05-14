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

    /// Three documents: two tive (sizes 1000, + 500) + one archived. Asserts
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
}
