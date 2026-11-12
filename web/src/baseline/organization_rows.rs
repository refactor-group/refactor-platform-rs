//! Organizations (`ORG`).

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::{
    holds_role, record_removal, relationships_between, user_exists, user_with_email,
};
use domain::test_utils::sqlite::{seed_membership, within_time_limit, Caller};
use serde_json::{json, Value};

use super::harness::{ids, Baseline};

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
async fn org_01_admins_add_a_member_with_a_coach() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut baseline = Baseline::new().await;
            let (o1, coach) = (baseline.world.o1, baseline.world.user(Caller::Coach));

            let reply = baseline
                .send(
                    caller,
                    Method::POST,
                    &format!("/organizations/{o1}/users"),
                    Some(new_member_body(coach)),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let user = user_with_email(baseline.db(), NEW_EMAIL)
                .await
                .expect("the member is stored");
            assert_eq!(reply.ids(), ids([user]), "{caller:?}");
            assert!(holds_role(baseline.db(), user, o1).await, "{caller:?}");
            assert_eq!(
                relationships_between(baseline.db(), coach, user)
                    .await
                    .len(),
                1,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ORG-02
#[tokio::test]
async fn org_02_non_admins_cannot_add_a_member() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (o1, coach) = (baseline.world.o1, baseline.world.user(Caller::Coach));

        for caller in [
            Caller::Coach,
            Caller::Coachee,
            Caller::Member,
            Caller::Outsider,
        ] {
            let (status, _) = baseline
                .expect(
                    caller,
                    Method::POST,
                    &format!("/organizations/{o1}/users"),
                    Some(new_member_body(coach)),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(
                user_with_email(baseline.db(), NEW_EMAIL).await,
                None,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ORG-03
#[tokio::test]
async fn org_03_admins_attach_a_member_with_a_coach() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut baseline = Baseline::new().await;
            let world = &baseline.world;
            let (o1, coach) = (world.o1, world.user(Caller::Coach));
            // A former O1 member, so ADMIN may see them.
            let target = world.user(Caller::Removed);
            record_removal(baseline.db(), target, o1).await;
            let before = relationships_between(baseline.db(), coach, target).await;

            let reply = baseline
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
            assert!(holds_role(baseline.db(), target, o1).await, "{caller:?}");
            assert_eq!(
                relationships_between(baseline.db(), coach, target).await,
                before,
                "{caller:?}: R3 is reused"
            );
        }
    })
    .await;
}

/// ORG-04
#[tokio::test]
async fn org_04_admins_remove_a_member_and_keep_their_relationship() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut baseline = Baseline::new().await;
            let world = &baseline.world;
            let (o1, r3, removed) = (world.o1, world.r3, world.user(Caller::Removed));
            let coach = world.user(Caller::Coach);
            seed_membership(baseline.db(), removed, o1).await;

            let reply = baseline
                .send(
                    caller,
                    Method::DELETE,
                    &format!("/organizations/{o1}/users/{removed}/role"),
                    None,
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 204, "{caller:?}");
            assert!(!holds_role(baseline.db(), removed, o1).await, "{caller:?}");
            assert_eq!(
                relationships_between(baseline.db(), coach, removed).await,
                ids([r3]),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// ORG-05
#[tokio::test]
async fn org_05_non_admins_cannot_remove_a_members_role() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (o1, member) = (baseline.world.o1, baseline.world.user(Caller::Member));
        let path = format!("/organizations/{o1}/users/{member}/role");

        for caller in [
            Caller::Coach,
            Caller::Coachee,
            Caller::Member,
            Caller::Outsider,
        ] {
            let (status, _) = baseline.expect(caller, Method::DELETE, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert!(holds_role(baseline.db(), member, o1).await, "{caller:?}");
        }
    })
    .await;
}

/// ORG-06
#[tokio::test]
async fn org_06_admins_delete_a_member_without_coaching_history() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut baseline = Baseline::new().await;
            let (o1, member) = (baseline.world.o1, baseline.world.user(Caller::Member));

            let reply = baseline
                .send(
                    caller,
                    Method::DELETE,
                    &format!("/organizations/{o1}/users/{member}"),
                    None,
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 204, "{caller:?}");
            assert!(!user_exists(baseline.db(), member).await, "{caller:?}");
        }
    })
    .await;
}

/// ORG-07
#[tokio::test]
async fn org_07_an_admin_cannot_delete_themselves() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let (o1, admin) = (baseline.world.o1, baseline.world.user(Caller::Admin));

        let (status, _) = baseline
            .expect(
                Caller::Admin,
                Method::DELETE,
                &format!("/organizations/{o1}/users/{admin}"),
                None,
            )
            .await;

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(user_exists(baseline.db(), admin).await);
        assert!(holds_role(baseline.db(), admin, o1).await);
    })
    .await;
}

/// ORG-08
#[tokio::test]
async fn org_08_deleting_a_member_with_coaching_history_is_refused() {
    within_time_limit(async {
        let mut baseline = Baseline::new().await;
        let world = &baseline.world;
        let (o1, coachee) = (world.o1, world.user(Caller::Coachee));
        let (coach, other_coach) = (world.user(Caller::Coach), world.user(Caller::OtherCoach));
        let relationships = ids([world.r1, world.r2]);

        for caller in [Caller::Admin, Caller::Super] {
            let reply = baseline
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
            assert!(user_exists(baseline.db(), coachee).await, "{caller:?}");
            assert!(holds_role(baseline.db(), coachee, o1).await, "{caller:?}");
            let kept: BTreeSet<_> = relationships_between(baseline.db(), coach, coachee)
                .await
                .union(&relationships_between(baseline.db(), other_coach, coachee).await)
                .copied()
                .collect();
            assert_eq!(kept, relationships, "{caller:?}");
        }
    })
    .await;
}
