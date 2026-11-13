//! Who can add, change and remove a member's role in an organization.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! How the routes behave once allowed is tested in `role_controller_tests.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::{record_removal, set_role};
use domain::test_utils::sqlite::read_back::{holds_role, relationships_between, role_in};
use domain::test_utils::sqlite::{seed_membership, within_time_limit, Caller};
use domain::users::Role;
use domain::Id;
use serde_json::json;

use crate::test_utils::sqlite::{ids, TestApp};

/// The role path for `user` in O1.
fn role_path(app: &TestApp, user: Id) -> String {
    format!("/organizations/{}/users/{user}/role", app.fixture.o1)
}

/// ORG-03
#[tokio::test]
async fn admins_attach_a_member_with_a_coach() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut app = TestApp::new().await;
            let fixture = &app.fixture;
            let (o1, coach) = (fixture.o1, fixture.user(Caller::Coach));
            // A former O1 member, so ADMIN may see them.
            let target = fixture.user(Caller::Removed);
            record_removal(app.db(), target, o1).await;
            let before = relationships_between(app.db(), coach, target).await;

            let reply = app
                .send(
                    caller,
                    Method::POST,
                    &format!("/organizations/{o1}/users/{target}/role"),
                    Some(json!({ "role": "User", "coach_id": coach })),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            assert_eq!(reply.ids(), ids([target]), "{caller:?}");
            assert!(holds_role(app.db(), target, o1).await, "{caller:?}");
            assert_eq!(
                relationships_between(app.db(), coach, target).await,
                before,
                "{caller:?}: R3 is reused"
            );
        }
    })
    .await;
}

/// ORG-04
#[tokio::test]
async fn admins_remove_a_member_and_keep_their_relationship() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut app = TestApp::new().await;
            let fixture = &app.fixture;
            let (o1, r3, removed) = (fixture.o1, fixture.r3, fixture.user(Caller::Removed));
            let coach = fixture.user(Caller::Coach);
            seed_membership(app.db(), removed, o1).await;

            let reply = app
                .send(
                    caller,
                    Method::DELETE,
                    &format!("/organizations/{o1}/users/{removed}/role"),
                    None,
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 204, "{caller:?}");
            assert!(!holds_role(app.db(), removed, o1).await, "{caller:?}");
            assert_eq!(
                relationships_between(app.db(), coach, removed).await,
                ids([r3]),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ORG-05
#[tokio::test]
async fn non_admins_cannot_remove_a_members_role() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, member) = (app.fixture.o1, app.fixture.user(Caller::Member));
        let path = format!("/organizations/{o1}/users/{member}/role");

        for caller in [
            Caller::Coach,
            Caller::Coachee,
            Caller::Member,
            Caller::Outsider,
        ] {
            let (status, _) = app.expect(caller, Method::DELETE, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert!(holds_role(app.db(), member, o1).await, "{caller:?}");
        }
    })
    .await;
}

/// ORG-16
#[tokio::test]
async fn an_admin_cannot_attach_a_user_they_cannot_see() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, outsider) = (app.fixture.o1, app.fixture.user(Caller::Outsider));
        let emails = app.expect_emails(0).await;

        let (status, _) = app
            .expect(
                Caller::Admin,
                Method::POST,
                &role_path(&app, outsider),
                Some(json!({ "role": "User" })),
            )
            .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(!holds_role(app.db(), outsider, o1).await);
        emails.assert_async().await;
    })
    .await;
}

/// ORG-17
#[tokio::test]
async fn an_admin_cannot_tell_a_missing_user_from_one_in_another_org() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let outsider = app.fixture.user(Caller::Outsider);

        let elsewhere = app
            .send(Caller::Admin, Method::GET, &role_path(&app, outsider), None)
            .await;
        let missing = app
            .send(
                Caller::Admin,
                Method::GET,
                &role_path(&app, Id::new_v4()),
                None,
            )
            .await;

        assert_eq!(elsewhere.status, StatusCode::NOT_FOUND);
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
        assert_eq!(elsewhere.bytes, missing.bytes);
    })
    .await;
}

/// ORG-18
#[tokio::test]
async fn an_admin_of_both_orgs_attaches_a_member_of_the_other() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let (o1, o2) = (fixture.o1, fixture.o2);
        let (admin, outsider) = (fixture.user(Caller::Admin), fixture.user(Caller::Outsider));
        set_role(app.db(), admin, Role::Admin, o2).await;

        let reply = app
            .send(
                Caller::Admin,
                Method::POST,
                &role_path(&app, outsider),
                Some(json!({ "role": "User" })),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        assert_eq!(reply.ids(), ids([outsider]));
        assert_eq!(role_in(app.db(), outsider, o1).await, Some(Role::User));
    })
    .await;
}

/// ORG-19
#[tokio::test]
async fn attaching_a_current_member_is_a_conflict() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, member) = (app.fixture.o1, app.fixture.user(Caller::Member));

        for caller in [Caller::Admin, Caller::Super] {
            let (status, _) = app
                .expect(
                    caller,
                    Method::POST,
                    &role_path(&app, member),
                    Some(json!({ "role": "Admin" })),
                )
                .await;
            assert_eq!(status, StatusCode::CONFLICT, "{caller:?}");
            assert_eq!(
                role_in(app.db(), member, o1).await,
                Some(Role::User),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ORG-20
#[tokio::test]
async fn an_admin_cannot_remove_their_own_role() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, admin) = (app.fixture.o1, app.fixture.user(Caller::Admin));

        let (status, _) = app
            .expect(Caller::Admin, Method::DELETE, &role_path(&app, admin), None)
            .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(role_in(app.db(), admin, o1).await, Some(Role::Admin));
    })
    .await;
}

/// ORG-21
#[tokio::test]
async fn non_admins_cannot_read_or_change_a_role() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, admin) = (app.fixture.o1, app.fixture.user(Caller::Admin));

        for caller in [
            Caller::Coach,
            Caller::Coachee,
            Caller::Member,
            Caller::Outsider,
        ] {
            for (method, body) in [
                (Method::GET, None),
                (Method::PUT, Some(json!({ "role": "User" }))),
            ] {
                let mut replies = Vec::new();
                for target in [admin, Id::new_v4()] {
                    let reply = app
                        .send(
                            caller,
                            method.clone(),
                            &role_path(&app, target),
                            body.clone(),
                        )
                        .await;
                    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{caller:?} {method}");
                    replies.push(reply.bytes);
                }
                assert_eq!(replies[0], replies[1], "{caller:?} {method}");
            }
            assert_eq!(
                role_in(app.db(), admin, o1).await,
                Some(Role::Admin),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ORG-22
#[tokio::test]
async fn an_admin_cannot_change_their_own_role() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, admin) = (app.fixture.o1, app.fixture.user(Caller::Admin));

        for role in ["Admin", "User"] {
            let (status, _) = app
                .expect(
                    Caller::Admin,
                    Method::PUT,
                    &role_path(&app, admin),
                    Some(json!({ "role": role })),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{role}");
            assert_eq!(
                role_in(app.db(), admin, o1).await,
                Some(Role::Admin),
                "{role}"
            );
        }
    })
    .await;
}

/// ORG-23
#[tokio::test]
async fn admins_read_a_members_role() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, member) = (app.fixture.o1, app.fixture.user(Caller::Member));

        for caller in [Caller::Admin, Caller::Super] {
            let reply = app
                .send(caller, Method::GET, &role_path(&app, member), None)
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            let data = &reply.body["data"];
            assert_eq!(data["role"], "User", "{caller:?}");
            assert_eq!(data["user_id"], member.to_string(), "{caller:?}");
            assert_eq!(data["organization_id"], o1.to_string(), "{caller:?}");
        }
    })
    .await;
}

/// ORG-24
#[tokio::test]
async fn admins_change_a_members_role() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut app = TestApp::new().await;
            let (o1, member) = (app.fixture.o1, app.fixture.user(Caller::Member));

            let reply = app
                .send(
                    caller,
                    Method::PUT,
                    &role_path(&app, member),
                    Some(json!({ "role": "Admin" })),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(
                role_in(app.db(), member, o1).await,
                Some(Role::Admin),
                "{caller:?}"
            );
        }
    })
    .await;
}
