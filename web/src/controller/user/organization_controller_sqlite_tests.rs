//! Who may list a user's organizations.
//!
//! Each test signs in as one kind of person, asks for the list against a real database, and
//! checks that only the user gets it.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::{within_time_limit, Caller};

use crate::test_utils::sqlite::{ids, TestApp};

/// Only the user lists their own organizations.
#[tokio::test]
async fn only_the_user_lists_their_own_organizations() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coachee = app.fixture.user(Caller::Coachee);
        let path = format!("/users/{coachee}/organizations");

        for caller in [Caller::Coach, Caller::Member, Caller::Admin, Caller::Super] {
            let reply = app.send(caller, Method::GET, &path, None).await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(&reply.bytes[..], b"FORBIDDEN", "{caller:?}");
        }

        let (status, organizations) = app.expect(Caller::Coachee, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(organizations, ids([app.fixture.o1]));
    })
    .await;
}
