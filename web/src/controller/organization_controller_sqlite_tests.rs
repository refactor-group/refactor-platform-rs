//! Who can create organizations.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed. Update, delete, archive and unarchive share the create route's
//! SuperAdmin check.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::set_scoped_super_admin;
use domain::test_utils::sqlite::read_back::organization_named;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use serde_json::{json, Value};

use crate::test_utils::sqlite::{ids, TestApp};

/// The organization ORG-25 to ORG-27 create.
const NEW_NAME: &str = "Kumquat Works";

fn create_body() -> Value {
    json!({ "name": NEW_NAME, "logo": null })
}

/// ORG-25
#[tokio::test]
async fn a_super_admin_creates_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;

        let reply = app
            .send(
                Caller::Super,
                Method::POST,
                "/organizations",
                Some(create_body()),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let created = organization_named(app.db(), NEW_NAME)
            .await
            .expect("the organization is stored");
        assert_eq!(reply.ids(), ids([created]));
    })
    .await;
}

/// ORG-26
#[tokio::test]
async fn only_a_super_admin_creates_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;

        for caller in [
            Caller::Admin,
            Caller::Coach,
            Caller::Member,
            Caller::Outsider,
        ] {
            let (status, _) = app
                .expect(caller, Method::POST, "/organizations", Some(create_body()))
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(
                organization_named(app.db(), NEW_NAME).await,
                None,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ORG-27
#[tokio::test]
async fn a_super_admin_role_held_in_one_org_cannot_create_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (outsider, o2) = (app.fixture.user(Caller::Outsider), app.fixture.o2);
        set_scoped_super_admin(app.db(), outsider, o2).await;

        let (status, _) = app
            .expect(
                Caller::Outsider,
                Method::POST,
                "/organizations",
                Some(create_body()),
            )
            .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(organization_named(app.db(), NEW_NAME).await, None);
    })
    .await;
}
