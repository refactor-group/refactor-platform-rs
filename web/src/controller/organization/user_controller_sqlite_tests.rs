//! Who can add and delete an organization's members.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::set_role;
use domain::test_utils::sqlite::read_back::{
    holds_role, relationships_between, role_in, user_exists, user_with_email,
};
use domain::test_utils::sqlite::{seed_membership, seed_user, within_time_limit, Caller};
use domain::users::Role;
use domain::Id;
use serde_json::{json, Value};

use crate::test_utils::sqlite::{assert_signed_out_refused, ids, TestApp};

/// The person ORG-01 and ORG-02 add.
const NEW_EMAIL: &str = "new@access.test";

/// A new O1 member coached by `coach`.
fn new_member_body(coach: domain::Id) -> Value {
    json!({
        "email": NEW_EMAIL,
        "first_name": "New",
        "last_name": "Member",
        "display_name": null,
        "password": "kumquat-new-password",
        "github_username": null,
        "github_profile_url": null,
        "coach_id": coach,
    })
}

/// Every O1 member but ADMIN, who administers it.
const NON_ADMINS: [Caller; 4] = [
    Caller::Coach,
    Caller::Coachee,
    Caller::Member,
    Caller::Outsider,
];

/// An O1 member who never finished setting up their account; returns their id and email.
async fn seed_pending_member(app: &TestApp) -> (Id, String) {
    let user = seed_user(app.db()).await;
    seed_membership(app.db(), user, app.fixture.o1).await;
    (user, format!("{user}@example.com"))
}

/// ORG-01
#[tokio::test]
async fn admins_add_a_member_with_a_coach() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut app = TestApp::new().await;
            let (o1, coach) = (app.fixture.o1, app.fixture.user(Caller::Coach));

            let reply = app
                .send(
                    caller,
                    Method::POST,
                    &format!("/organizations/{o1}/users"),
                    Some(new_member_body(coach)),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let user = user_with_email(app.db(), NEW_EMAIL)
                .await
                .expect("the member is stored");
            assert_eq!(reply.ids(), ids([user]), "{caller:?}");
            assert!(holds_role(app.db(), user, o1).await, "{caller:?}");
            assert_eq!(
                relationships_between(app.db(), coach, user).await.len(),
                1,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ORG-02
#[tokio::test]
async fn non_admins_cannot_add_a_member() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, coach) = (app.fixture.o1, app.fixture.user(Caller::Coach));

        for caller in [
            Caller::Coach,
            Caller::Coachee,
            Caller::Member,
            Caller::Outsider,
        ] {
            let (status, _) = app
                .expect(
                    caller,
                    Method::POST,
                    &format!("/organizations/{o1}/users"),
                    Some(new_member_body(coach)),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(
                user_with_email(app.db(), NEW_EMAIL).await,
                None,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ORG-06
#[tokio::test]
async fn admins_delete_a_member_without_coaching_history() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut app = TestApp::new().await;
            let (o1, member) = (app.fixture.o1, app.fixture.user(Caller::Member));

            let reply = app
                .send(
                    caller,
                    Method::DELETE,
                    &format!("/organizations/{o1}/users/{member}"),
                    None,
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 204, "{caller:?}");
            assert!(!user_exists(app.db(), member).await, "{caller:?}");
        }
    })
    .await;
}

/// ORG-07
#[tokio::test]
async fn an_admin_cannot_delete_themselves() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, admin) = (app.fixture.o1, app.fixture.user(Caller::Admin));

        let (status, _) = app
            .expect(
                Caller::Admin,
                Method::DELETE,
                &format!("/organizations/{o1}/users/{admin}"),
                None,
            )
            .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(user_exists(app.db(), admin).await);
        assert!(holds_role(app.db(), admin, o1).await);
    })
    .await;
}

/// ORG-08
#[tokio::test]
async fn deleting_a_member_with_coaching_history_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let (o1, coachee) = (fixture.o1, fixture.user(Caller::Coachee));
        let (coach, other_coach) = (
            fixture.user(Caller::Coach),
            fixture.user(Caller::OtherCoach),
        );
        let relationships = ids([fixture.r1, fixture.r2]);

        for caller in [Caller::Admin, Caller::Super] {
            let reply = app
                .send(
                    caller,
                    Method::DELETE,
                    &format!("/organizations/{o1}/users/{coachee}"),
                    None,
                )
                .await;

            assert_eq!(reply.status, StatusCode::CONFLICT, "{caller:?}");
            assert_eq!(
                reply.body["details"]["reason"], "user_has_coaching_history",
                "{caller:?}"
            );
            assert_eq!(
                reply.body["details"]["user_id"],
                coachee.to_string(),
                "{caller:?}"
            );
            assert_eq!(reply.body["details"]["relationship_count"], 2, "{caller:?}");
            assert!(user_exists(app.db(), coachee).await, "{caller:?}");
            assert!(holds_role(app.db(), coachee, o1).await, "{caller:?}");
            let kept: BTreeSet<_> = relationships_between(app.db(), coach, coachee)
                .await
                .union(&relationships_between(app.db(), other_coach, coachee).await)
                .copied()
                .collect();
            assert_eq!(kept, relationships, "{caller:?}");
        }
    })
    .await;
}

/// ORG-09
#[tokio::test]
async fn non_admins_cannot_resend_a_members_invite() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (pending, _) = seed_pending_member(&app).await;
        let path = format!(
            "/organizations/{}/users/{pending}/resend-invite",
            app.fixture.o1
        );
        let emails = app.expect_emails(0).await;

        for caller in NON_ADMINS {
            let (status, _) = app.expect(caller, Method::POST, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        emails.assert_async().await;
    })
    .await;
}

/// ORG-10
#[tokio::test]
async fn admins_resend_a_pending_members_invite() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut app = TestApp::new().await;
            let (pending, email) = seed_pending_member(&app).await;
            app.take_emailed();

            let reply = app
                .send(
                    caller,
                    Method::POST,
                    &format!(
                        "/organizations/{}/users/{pending}/resend-invite",
                        app.fixture.o1
                    ),
                    None,
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.ids(), ids([pending]), "{caller:?}");
            assert_eq!(app.take_emailed(), vec![email], "{caller:?}");
        }
    })
    .await;
}

/// ORG-11
#[tokio::test]
async fn non_admins_cannot_delete_a_member() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, member) = (app.fixture.o1, app.fixture.user(Caller::Member));
        let path = format!("/organizations/{o1}/users/{member}");

        for caller in NON_ADMINS {
            let (status, _) = app.expect(caller, Method::DELETE, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert!(user_exists(app.db(), member).await, "{caller:?}");
            assert!(holds_role(app.db(), member, o1).await, "{caller:?}");
        }
    })
    .await;
}

/// ORG-12
#[tokio::test]
async fn an_admin_of_another_org_cannot_manage_its_members() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let (o1, o2) = (fixture.o1, fixture.o2);
        let (coach, member, outsider) = (
            fixture.user(Caller::Coach),
            fixture.user(Caller::Member),
            fixture.user(Caller::Outsider),
        );
        set_role(app.db(), outsider, Role::Admin, o2).await;
        let emails = app.expect_emails(0).await;
        let users = format!("/organizations/{o1}/users");
        let mut refusals = Vec::new();

        for target in [member, Id::new_v4()] {
            for (method, path, body) in [
                (Method::POST, users.clone(), Some(new_member_body(coach))),
                (
                    Method::POST,
                    format!("{users}/{target}/resend-invite"),
                    None,
                ),
                (Method::DELETE, format!("{users}/{target}"), None),
                (Method::GET, format!("{users}/{target}/role"), None),
                (
                    Method::POST,
                    format!("{users}/{target}/role"),
                    Some(json!({ "role": "User" })),
                ),
                (
                    Method::PUT,
                    format!("{users}/{target}/role"),
                    Some(json!({ "role": "Admin" })),
                ),
                (Method::DELETE, format!("{users}/{target}/role"), None),
            ] {
                let reply = app
                    .send(Caller::Outsider, method.clone(), &path, body)
                    .await;
                assert_eq!(reply.status, StatusCode::FORBIDDEN, "{method} {path}");
                refusals.push(reply.bytes);
            }
        }
        let unknown_org = app
            .send(
                Caller::Outsider,
                Method::POST,
                &format!("/organizations/{}/users", Id::new_v4()),
                Some(new_member_body(coach)),
            )
            .await;
        assert_eq!(unknown_org.status, StatusCode::FORBIDDEN);
        refusals.push(unknown_org.bytes);

        assert!(
            refusals.iter().all(|bytes| *bytes == refusals[0]),
            "a real and a fabricated target or organization are refused alike"
        );
        assert_eq!(user_with_email(app.db(), NEW_EMAIL).await, None);
        assert!(user_exists(app.db(), member).await);
        assert_eq!(role_in(app.db(), member, o1).await, Some(Role::User));
        emails.assert_async().await;
    })
    .await;
}

/// ORG-13
#[tokio::test]
async fn an_unknown_organization_is_not_found() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coach = app.fixture.user(Caller::Coach);

        let (status, _) = app
            .expect(
                Caller::Super,
                Method::POST,
                &format!("/organizations/{}/users", Id::new_v4()),
                Some(new_member_body(coach)),
            )
            .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(user_with_email(app.db(), NEW_EMAIL).await, None);
    })
    .await;
}

/// ORG-14
#[tokio::test]
async fn a_non_admin_cannot_tell_whether_an_organization_exists() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coach = app.fixture.user(Caller::Coach);

        for caller in [Caller::Member, Caller::Admin] {
            let mut replies = Vec::new();
            for organization in [app.fixture.o2, Id::new_v4()] {
                let reply = app
                    .send(
                        caller,
                        Method::POST,
                        &format!("/organizations/{organization}/users"),
                        Some(new_member_body(coach)),
                    )
                    .await;
                assert_eq!(reply.status, StatusCode::FORBIDDEN, "{caller:?}");
                replies.push(reply.bytes);
            }
            assert_eq!(replies[0], replies[1], "{caller:?}");
        }
        assert_eq!(user_with_email(app.db(), NEW_EMAIL).await, None);
    })
    .await;
}

/// ORG-15
#[tokio::test]
async fn a_user_outside_the_organization_is_not_found() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, o2, outsider) = (
            app.fixture.o1,
            app.fixture.o2,
            app.fixture.user(Caller::Outsider),
        );

        for caller in [Caller::Admin, Caller::Super] {
            for target in [outsider, Id::new_v4()] {
                let (status, _) = app
                    .expect(
                        caller,
                        Method::DELETE,
                        &format!("/organizations/{o1}/users/{target}"),
                        None,
                    )
                    .await;
                assert_eq!(status, StatusCode::NOT_FOUND, "{caller:?} {target}");
            }
        }
        assert!(user_exists(app.db(), outsider).await);
        assert!(holds_role(app.db(), outsider, o2).await);
    })
    .await;
}

/// ORG-28
#[tokio::test]
async fn signed_out_callers_cannot_reach_organizations() {
    within_time_limit(assert_signed_out_refused(|app| {
        let (o1, member) = (app.fixture.o1, app.fixture.user(Caller::Member));
        let users = format!("/organizations/{o1}/users");
        vec![
            (Method::GET, "/organizations".to_string()),
            (Method::POST, "/organizations".to_string()),
            (Method::GET, format!("/organizations/{o1}")),
            (Method::GET, users.clone()),
            (Method::POST, users.clone()),
            (Method::POST, format!("{users}/{member}/resend-invite")),
            (Method::DELETE, format!("{users}/{member}")),
            (Method::GET, format!("{users}/{member}/role")),
            (Method::PUT, format!("{users}/{member}/role")),
        ]
    }))
    .await;
}
