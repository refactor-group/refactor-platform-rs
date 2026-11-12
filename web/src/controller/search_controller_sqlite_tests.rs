//! Who finds what in search.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! Searches that find something need Postgres full-text search, so only searches that find
//! nothing run here; the rest are checked by hand.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};

use crate::test_utils::sqlite::{ids_in, TestApp};

/// Asserts each caller's search on `path` answers 200 with no hits.
async fn assert_no_hits(app: &mut TestApp, callers: &[Caller], path: &str) {
    for &caller in callers {
        let reply = app.send(caller, Method::GET, path, None).await;

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
async fn members_outside_every_relationship_find_nothing() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        assert_no_hits(
            &mut app,
            &[Caller::Member, Caller::Admin, Caller::Outsider],
            "/search?q=kumquat&types=actions",
        )
        .await;
    })
    .await;
}

/// SRCH-06
#[tokio::test]
async fn a_removed_coachee_finds_nothing() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        assert_no_hits(
            &mut app,
            &[Caller::Removed],
            "/search?q=kumquat&types=actions",
        )
        .await;
    })
    .await;
}

/// SRCH-08
#[tokio::test]
async fn a_relationship_filter_never_widens_a_members_search() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!(
            "/search?q=kumquat&types=actions&coaching_relationship_id={}",
            app.fixture.r1
        );
        assert_no_hits(&mut app, &[Caller::Member], &path).await;
    })
    .await;
}
