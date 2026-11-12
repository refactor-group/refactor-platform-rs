//! Organization member roles (`ORG`), on SQLite as each access-matrix caller.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::{holds_role, record_removal, relationships_between};
use domain::test_utils::sqlite::{seed_membership, within_time_limit, Caller};
use serde_json::json;

use crate::test_utils::sqlite::{ids, TestApp};

/// ORG-03
#[tokio::test]
async fn admins_attach_a_member_with_a_coach() {
    within_time_limit(async {
        for caller in [Caller::Admin, Caller::Super] {
            let mut app = TestApp::new().await;
            let world = &app.world;
            let (o1, coach) = (world.o1, world.user(Caller::Coach));
            // A former O1 member, so ADMIN may see them.
            let target = world.user(Caller::Removed);
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
            let world = &app.world;
            let (o1, r3, removed) = (world.o1, world.r3, world.user(Caller::Removed));
            let coach = world.user(Caller::Coach);
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
        let (o1, member) = (app.world.o1, app.world.user(Caller::Member));
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
