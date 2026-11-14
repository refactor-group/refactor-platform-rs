//! Who may change a user's password.
//!
//! Each test signs in as one kind of person, calls the route against a real database, and
//! checks that everyone but the user is refused with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::PASSWORD;
use domain::test_utils::sqlite::read_back::password_hash;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use serde_json::json;

use crate::test_utils::sqlite::TestApp;

/// Only the user changes their own password.
#[tokio::test]
async fn only_the_user_changes_their_own_password() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coachee = app.fixture.user(Caller::Coachee);
        let path = format!("/users/{coachee}/password");
        let body = json!({
            "current_password": PASSWORD,
            "new_password": "persimmon-password",
            "confirm_password": "persimmon-password",
        });
        let before = password_hash(app.db(), coachee).await;

        for caller in [Caller::Coach, Caller::Member, Caller::Admin, Caller::Super] {
            let reply = app
                .send(caller, Method::PUT, &path, Some(body.clone()))
                .await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(&reply.bytes[..], b"FORBIDDEN", "{caller:?}");
            assert_eq!(password_hash(app.db(), coachee).await, before, "{caller:?}");
        }
    })
    .await;
}
