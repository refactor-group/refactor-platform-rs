//! Search (`SRCH`). Rows with hits need Postgres full-text search; only the rows with no
//! hits run here.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};

use super::harness::{ids_in, Baseline};

/// Asserts each caller's search on `path` answers 200 with no hits.
async fn assert_no_hits(baseline: &mut Baseline, callers: &[Caller], path: &str) {
    for &caller in callers {
        let reply = baseline.send(caller, Method::GET, path, None).await;

        assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
        assert_eq!(
            ids_in(&reply.body["data"]["hits"]),
            BTreeSet::new(),
            "{caller:?}"
        );
    }
}

/// SRCH-04
#[tokio::test]
async fn srch_04_members_outside_every_relationship_find_nothing() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        assert_no_hits(
            &mut baseline,
            &[Caller::Member, Caller::Admin, Caller::Outsider],
            "/search?q=kumquat&types=actions",
        )
        .await;
    })
    .await;
}

/// SRCH-06
#[tokio::test]
async fn srch_06_a_removed_coachee_finds_nothing() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        assert_no_hits(
            &mut baseline,
            &[Caller::Removed],
            "/search?q=kumquat&types=actions",
        )
        .await;
    })
    .await;
}

/// SRCH-08
#[tokio::test]
async fn srch_08_a_relationship_filter_never_widens_a_members_search() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let path = format!(
            "/search?q=kumquat&types=actions&coaching_relationship_id={}",
            baseline.world.r1
        );
        assert_no_hits(&mut baseline, &[Caller::Member], &path).await;
    })
    .await;
}
