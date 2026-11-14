//! Who may read the Tiptap document metrics.
//!
//! Each test signs in as one kind of person and calls the metrics routes.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};

use crate::test_utils::sqlite::TestApp;

const METRICS: [&str; 3] = [
    "/admin/tiptap/metrics/totals",
    "/admin/tiptap/metrics/per-org",
    "/admin/tiptap/metrics/abandoned",
];

/// Only a SuperAdmin reads the metrics.
#[tokio::test]
async fn only_a_super_admin_reads_the_metrics() {
    within_time_limit(async {
        let mut app = TestApp::new().await;

        for path in METRICS {
            for caller in [Caller::Coach, Caller::Admin, Caller::Member] {
                let reply = app.send(caller, Method::GET, path, None).await;
                assert_eq!(reply.status, StatusCode::FORBIDDEN, "{caller:?} {path}");
            }
            // Past the check, the Tiptap stand-in has no document list, so the handler answers 502.
            let reply = app.send(Caller::Super, Method::GET, path, None).await;
            assert_eq!(reply.status, StatusCode::BAD_GATEWAY, "{path}");
        }
    })
    .await;
}
