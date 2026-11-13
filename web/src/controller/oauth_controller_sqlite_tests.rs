//! Who can see and remove meeting provider connections.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::seed_google_connection;
use domain::test_utils::sqlite::read_back::connected_users;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use serde_json::{json, Value};

use crate::test_utils::sqlite::{assert_signed_out_refused, ids, TestApp};

/// The connected account emails `caller` lists.
async fn listed_emails(app: &mut TestApp, caller: Caller) -> BTreeSet<String> {
    let reply = app
        .send(caller, Method::GET, "/oauth/connections", None)
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
    reply.body["data"]
        .as_array()
        .expect("a list of connections")
        .iter()
        .filter_map(|connection| connection["email"].as_str().map(str::to_string))
        .collect()
}

/// MEET-05
#[tokio::test]
async fn every_caller_reaches_only_their_own_connections() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (coach, other_coach) = (
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::OtherCoach),
        );
        seed_google_connection(app.db(), coach, "coach@google.test").await;
        seed_google_connection(app.db(), other_coach, "other@google.test").await;

        for caller in Caller::ALL {
            let own: BTreeSet<String> = match caller {
                Caller::Coach => ["coach@google.test".to_string()].into(),
                Caller::OtherCoach => ["other@google.test".to_string()].into(),
                _ => BTreeSet::new(),
            };
            assert_eq!(listed_emails(&mut app, caller).await, own, "{caller:?}");

            let reply = app
                .send(caller, Method::GET, "/oauth/connections/google", None)
                .await;
            let expected = match own.first() {
                Some(email) => (StatusCode::OK, json!(email)),
                None => (StatusCode::NOT_FOUND, Value::Null),
            };
            assert_eq!(
                (reply.status, reply.body["data"]["email"].clone()),
                expected,
                "{caller:?}"
            );
        }

        // A caller with no connection deletes nothing of anyone else's.
        let reply = app
            .send(
                Caller::Member,
                Method::DELETE,
                "/oauth/connections/google",
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
        assert_eq!(connected_users(app.db()).await, ids([coach, other_coach]));

        let reply = app
            .send(
                Caller::Coach,
                Method::DELETE,
                "/oauth/connections/google",
                None,
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(connected_users(app.db()).await, ids([other_coach]));
    })
    .await;
}

/// MEET-06
#[tokio::test]
async fn signed_out_callers_cannot_reach_connections() {
    within_time_limit(assert_signed_out_refused(|_| {
        vec![
            (Method::GET, "/oauth/google/authorize".to_string()),
            (Method::GET, "/oauth/connections".to_string()),
            (Method::GET, "/oauth/connections/google".to_string()),
            (Method::DELETE, "/oauth/connections/google".to_string()),
        ]
    }))
    .await;
}
