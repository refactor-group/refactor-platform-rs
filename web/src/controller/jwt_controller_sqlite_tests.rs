//! Who can get a token to edit a session's shared notes.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;

use crate::test_utils::sqlite::{TestApp, NON_PARTICIPANTS};

/// The token path for `session`.
fn token_path(session: Id) -> String {
    format!("/jwt/generate_collab_token?coaching_session_id={session}")
}

/// JWT-01
#[tokio::test]
async fn participants_get_a_token_for_the_sessions_document() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.world.s1;

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = app.send(caller, Method::GET, &token_path(s1), None).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            let document = reply.body["data"]["sub"].as_str().unwrap_or_default();
            assert!(document.contains(&s1.to_string()), "{caller:?}: {document}");
        }
    })
    .await;
}

/// JWT-02
#[tokio::test]
async fn non_participants_get_no_token() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.world.s1;

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &token_path(s1), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }

        // The matrix's note: an unknown session is not found.
        let (status, _) = app
            .expect(Caller::Coach, Method::GET, &token_path(Id::new_v4()), None)
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await;
}

/// JWT-03
#[tokio::test]
async fn a_removed_coachee_gets_no_token() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s3 = app.world.s3;

        let (status, _) = app
            .expect(Caller::Removed, Method::GET, &token_path(s3), None)
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}
