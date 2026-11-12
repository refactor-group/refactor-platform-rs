//! Who can add and delete an organization's members.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! How the routes behave once allowed is tested in `user_controller_tests.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::{
    holds_role, relationships_between, user_exists, user_with_email,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use serde_json::{json, Value};

use crate::test_utils::sqlite::{ids, TestApp};

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

/// ORG-01
#[tokio::test]
async fn admins_add_a_member_with_a_coach() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut app = TestApp::new().await;
            let (o1, coach) = (app.world.o1, app.world.user(Caller::Coach));

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
        let (o1, coach) = (app.world.o1, app.world.user(Caller::Coach));

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
            let (o1, member) = (app.world.o1, app.world.user(Caller::Member));

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
        let (o1, admin) = (app.world.o1, app.world.user(Caller::Admin));

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
        let world = &app.world;
        let (o1, coachee) = (world.o1, world.user(Caller::Coachee));
        let (coach, other_coach) = (world.user(Caller::Coach), world.user(Caller::OtherCoach));
        let relationships = ids([world.r1, world.r2]);

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
