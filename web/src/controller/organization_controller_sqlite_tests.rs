//! Who can read, create, update, delete, archive and unarchive organizations.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::{archive_organization, set_scoped_super_admin};
use domain::test_utils::sqlite::read_back::{organization_named, organization_state};
use domain::test_utils::sqlite::{seed_organization, within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use crate::test_utils::sqlite::{ids, TestApp};

/// The organization ORG-25 to ORG-27 create.
const NEW_NAME: &str = "Kumquat Works";

fn create_body() -> Value {
    json!({ "name": NEW_NAME, "logo": null })
}

/// Asserts ADMIN, COACH, MEMBER and OUTSIDER, then OUTSIDER holding a SuperAdmin role scoped
/// to O2, are each refused the request.
async fn assert_only_a_super_admin(
    app: &mut TestApp,
    method: Method,
    path: &str,
    body: Option<Value>,
) {
    for caller in [
        Caller::Admin,
        Caller::Coach,
        Caller::Member,
        Caller::Outsider,
    ] {
        let (status, _) = app.expect(caller, method.clone(), path, body.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
    }
    let (outsider, o2) = (app.fixture.user(Caller::Outsider), app.fixture.o2);
    set_scoped_super_admin(app.db(), outsider, o2).await;
    let (status, _) = app.expect(Caller::Outsider, method, path, body).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "scoped SuperAdmin");
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

/// ORG-29
#[tokio::test]
async fn a_super_admin_renames_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let o1 = app.fixture.o1;

        let reply = app
            .send(
                Caller::Super,
                Method::PUT,
                &format!("/organizations/{o1}"),
                Some(create_body()),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.ids(), ids([o1]));
        assert_eq!(
            organization_state(app.db(), o1).await,
            Some((NEW_NAME.to_string(), false))
        );
    })
    .await;
}

/// ORG-30
#[tokio::test]
async fn only_a_super_admin_renames_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let o1 = app.fixture.o1;
        let before = organization_state(app.db(), o1).await;

        assert_only_a_super_admin(
            &mut app,
            Method::PUT,
            &format!("/organizations/{o1}"),
            Some(create_body()),
        )
        .await;

        assert_eq!(organization_state(app.db(), o1).await, before);
    })
    .await;
}

/// ORG-31
#[tokio::test]
async fn a_super_admin_deletes_an_empty_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let empty = seed_organization(app.db()).await;

        let reply = app
            .send(
                Caller::Super,
                Method::DELETE,
                &format!("/organizations/{empty}"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["id"], json!(empty));
        assert_eq!(organization_state(app.db(), empty).await, None);
    })
    .await;
}

/// ORG-32
#[tokio::test]
async fn only_a_super_admin_deletes_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let empty = seed_organization(app.db()).await;
        let before = organization_state(app.db(), empty).await;
        assert!(before.is_some());

        assert_only_a_super_admin(
            &mut app,
            Method::DELETE,
            &format!("/organizations/{empty}"),
            None,
        )
        .await;

        assert_eq!(organization_state(app.db(), empty).await, before);
    })
    .await;
}

/// ORG-33
#[tokio::test]
async fn a_super_admin_archives_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let o1 = app.fixture.o1;
        let (name, _) = organization_state(app.db(), o1)
            .await
            .expect("O1 is stored");

        let reply = app
            .send(
                Caller::Super,
                Method::POST,
                &format!("/organizations/{o1}/archive"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.ids(), ids([o1]));
        assert_eq!(organization_state(app.db(), o1).await, Some((name, true)));
    })
    .await;
}

/// ORG-34
#[tokio::test]
async fn only_a_super_admin_archives_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let o1 = app.fixture.o1;
        let before = organization_state(app.db(), o1).await;

        assert_only_a_super_admin(
            &mut app,
            Method::POST,
            &format!("/organizations/{o1}/archive"),
            None,
        )
        .await;

        assert_eq!(organization_state(app.db(), o1).await, before);
    })
    .await;
}

/// ORG-35
#[tokio::test]
async fn a_super_admin_unarchives_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, super_admin) = (app.fixture.o1, app.fixture.user(Caller::Super));
        archive_organization(app.db(), o1, super_admin).await;
        let (name, archived) = organization_state(app.db(), o1)
            .await
            .expect("O1 is stored");
        assert!(archived);

        let reply = app
            .send(
                Caller::Super,
                Method::POST,
                &format!("/organizations/{o1}/unarchive"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.ids(), ids([o1]));
        assert_eq!(organization_state(app.db(), o1).await, Some((name, false)));
    })
    .await;
}

/// ORG-36
#[tokio::test]
async fn only_a_super_admin_unarchives_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, super_admin) = (app.fixture.o1, app.fixture.user(Caller::Super));
        archive_organization(app.db(), o1, super_admin).await;
        let before = organization_state(app.db(), o1).await;

        assert_only_a_super_admin(
            &mut app,
            Method::POST,
            &format!("/organizations/{o1}/unarchive"),
            None,
        )
        .await;

        assert_eq!(organization_state(app.db(), o1).await, before);
    })
    .await;
}

/// ORG-37
#[tokio::test]
async fn members_and_super_admins_read_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let o1 = app.fixture.o1;

        for caller in [
            Caller::Coach,
            Caller::Coachee,
            Caller::OtherCoach,
            Caller::Member,
            Caller::Admin,
            Caller::Super,
        ] {
            let (status, ids_returned) = app
                .expect(caller, Method::GET, &format!("/organizations/{o1}"), None)
                .await;
            assert_eq!(status, StatusCode::OK, "{caller:?}");
            assert_eq!(ids_returned, ids([o1]), "{caller:?}");
        }
    })
    .await;
}

/// ORG-38
#[tokio::test]
async fn non_members_cannot_read_an_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let o1 = app.fixture.o1;

        for caller in [Caller::Outsider, Caller::Removed] {
            let (status, _) = app
                .expect(caller, Method::GET, &format!("/organizations/{o1}"), None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// ORG-39
#[tokio::test]
async fn an_unknown_organization_is_not_found() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let unknown = Id::new_v4();

        let (status, _) = app
            .expect(
                Caller::Super,
                Method::GET,
                &format!("/organizations/{unknown}"),
                None,
            )
            .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await;
}
