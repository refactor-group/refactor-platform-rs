//! Who can add and delete an organization's members.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use domain::coaching_relationship_kind::Kind;
use domain::test_utils::sqlite::fixture::set_role;
use domain::test_utils::sqlite::read_back::{
    holds_role, relationships_between, role_in, user_exists, user_with_email, verified_audit_trail,
    AuditEntry,
};
use domain::test_utils::sqlite::{seed_membership, seed_user, within_time_limit, Caller};
use domain::users::Role;
use domain::Id;
use serde_json::{json, Value};

use crate::test_utils::sqlite::{assert_signed_out_refused, created_id, ids, TestApp};

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
        "relationship": {"with_user_id": coach, "kind": "coaching"},
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

/// Adding a member with a coach stores a coaching relationship.
#[tokio::test]
async fn adding_a_member_with_a_coach_stores_a_coaching_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, coach) = (app.fixture.o1, app.fixture.user(Caller::Coach));

        let reply = app
            .send(
                Caller::Admin,
                Method::POST,
                &format!("/organizations/{o1}/users"),
                Some(new_member_body(coach)),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        let user = user_with_email(app.db(), NEW_EMAIL)
            .await
            .expect("the member is stored");
        assert_eq!(
            relationships_between(app.db(), coach, user)
                .await
                .into_iter()
                .map(|relationship| relationship.kind)
                .collect::<Vec<_>>(),
            vec![Kind::Coaching]
        );
    })
    .await;
}

/// KIND-15
#[tokio::test]
async fn admins_add_a_member_as_a_colleague() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, coach) = (app.fixture.o1, app.fixture.user(Caller::Coach));
        let mut body = new_member_body(coach);
        body["relationship"]["kind"] = json!("colleague");

        let reply = app
            .send(
                Caller::Admin,
                Method::POST,
                &format!("/organizations/{o1}/users"),
                Some(body),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let user = user_with_email(app.db(), NEW_EMAIL)
            .await
            .expect("the member is stored");
        assert_eq!(
            relationships_between(app.db(), coach, user)
                .await
                .into_iter()
                .map(|relationship| relationship.kind)
                .collect::<Vec<_>>(),
            vec![Kind::Colleague]
        );
        assert!(relationships_between(app.db(), user, coach)
            .await
            .is_empty());
        assert_eq!(app.take_emailed(), vec![NEW_EMAIL.to_string()]);
    })
    .await;
}

/// KIND-18
#[tokio::test]
async fn a_refused_relationship_adds_no_member_and_sends_no_email() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let o1 = app.fixture.o1;
        // Not an O1 member, so no relationship with them can be made there.
        let stranger = seed_user(app.db()).await;
        let mut body = new_member_body(stranger);
        body["relationship"]["kind"] = json!("colleague");
        app.take_emailed();

        let reply = app
            .send(
                Caller::Admin,
                Method::POST,
                &format!("/organizations/{o1}/users"),
                Some(body),
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_eq!(user_with_email(app.db(), NEW_EMAIL).await, None);
        assert_eq!(app.take_emailed(), Vec::<String>::new());
    })
    .await;
}

/// ORG-41
#[tokio::test]
async fn admins_add_a_member_as_the_coach() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, coachee) = (app.fixture.o1, app.fixture.user(Caller::Coachee));
        let mut body = new_member_body(coachee);
        body["relationship"]["role"] = json!("coach");
        app.take_emailed();

        let reply = app
            .send(
                Caller::Admin,
                Method::POST,
                &format!("/organizations/{o1}/users"),
                Some(body),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let user = user_with_email(app.db(), NEW_EMAIL)
            .await
            .expect("the member is stored");
        assert_eq!(
            relationships_between(app.db(), user, coachee)
                .await
                .into_iter()
                .map(|relationship| relationship.kind)
                .collect::<Vec<_>>(),
            vec![Kind::Coaching]
        );
        assert!(relationships_between(app.db(), coachee, user)
            .await
            .is_empty());
        assert_eq!(app.take_emailed(), vec![NEW_EMAIL.to_string()]);
    })
    .await;
}

/// ORG-42
#[tokio::test]
async fn a_role_the_kind_lacks_adds_no_member_and_sends_no_email() {
    within_time_limit(async {
        for (kind, role) in [("coaching", "colleague"), ("colleague", "coach")] {
            let mut app = TestApp::new().await;
            let (o1, coach) = (app.fixture.o1, app.fixture.user(Caller::Coach));
            let mut body = new_member_body(coach);
            body["relationship"]["kind"] = json!(kind);
            body["relationship"]["role"] = json!(role);
            app.take_emailed();

            let reply = app
                .send(
                    Caller::Admin,
                    Method::POST,
                    &format!("/organizations/{o1}/users"),
                    Some(body),
                )
                .await;

            assert_eq!(
                reply.status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{kind} {role}"
            );
            assert_eq!(
                reply.body["details"]["reason"], "role_not_in_kind",
                "{kind} {role}"
            );
            assert_eq!(
                user_with_email(app.db(), NEW_EMAIL).await,
                None,
                "{kind} {role}"
            );
            assert_eq!(app.take_emailed(), Vec::<String>::new(), "{kind} {role}");
        }
    })
    .await;
}

/// ORG-40
#[tokio::test]
async fn a_member_body_with_the_replaced_coach_id_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, coach) = (app.fixture.o1, app.fixture.user(Caller::Coach));
        let mut body = new_member_body(coach);
        body["coach_id"] = json!(coach);
        app.take_emailed();

        let reply = app
            .send(
                Caller::Admin,
                Method::POST,
                &format!("/organizations/{o1}/users"),
                Some(body),
            )
            .await;

        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        // The rejection an unknown field in any other body gets.
        let message = reply.body.as_str().unwrap_or_default();
        assert!(
            message.starts_with("Failed to deserialize the JSON body into the target type")
                && message.contains("unknown field `coach_id`"),
            "{message}"
        );
        assert_eq!(user_with_email(app.db(), NEW_EMAIL).await, None);
        assert_eq!(app.take_emailed(), Vec::<String>::new());
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
                .into_iter()
                .chain(relationships_between(app.db(), other_coach, coachee).await)
                .map(|relationship| relationship.id)
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

/// The `session_label` a pending member's resent invite carries once they have one
/// relationship of `kind` in O1 with COACH.
async fn resent_invites_session_label(kind: &str) -> Value {
    let mut app = TestApp::new().await;
    let o1 = app.fixture.o1;
    let (pending, email) = seed_pending_member(&app).await;
    let coach = app.fixture.user(Caller::Coach);
    let created = app
        .send(
            Caller::Admin,
            Method::POST,
            &format!("/organizations/{o1}/coaching_relationships"),
            Some(json!({ "coach_id": coach, "coachee_id": pending, "kind": kind })),
        )
        .await;
    assert_eq!(created.body["status_code"], 201, "{}", created.body);
    app.take_emailed();

    let reply = app
        .send(
            Caller::Admin,
            Method::POST,
            &format!("/organizations/{o1}/users/{pending}/resend-invite"),
            None,
        )
        .await;

    assert_eq!(reply.status, StatusCode::OK);
    let [(to, variables)] =
        <[_; 1]>::try_from(app.take_emailed_variables()).expect("one invite is resent");
    assert_eq!(to, email);
    variables["session_label"].clone()
}

/// A resent invite names sessions by the member's colleague relationship.
#[tokio::test]
async fn a_colleagues_resent_invite_says_session() {
    within_time_limit(async {
        assert_eq!(resent_invites_session_label("colleague").await, "session");
    })
    .await;
}

/// A resent invite names sessions by the member's coaching relationship.
#[tokio::test]
async fn a_coachees_resent_invite_says_coaching_session() {
    within_time_limit(async {
        assert_eq!(
            resent_invites_session_label("coaching").await,
            "coaching session"
        );
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

/// ORG-45
#[tokio::test]
async fn adding_a_member_records_the_membership_and_the_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, coach) = (app.fixture.o1, app.fixture.user(Caller::Coach));
        let admin = app.fixture.user(Caller::Admin);
        let mut body = new_member_body(coach);
        body["relationship"]["kind"] = json!("colleague");

        let reply = app
            .send(
                Caller::Admin,
                Method::POST,
                &format!("/organizations/{o1}/users"),
                Some(body),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        let user = user_with_email(app.db(), NEW_EMAIL)
            .await
            .expect("the member is stored");
        let relationship = relationships_between(app.db(), coach, user)
            .await
            .pop()
            .expect("the relationship is stored");
        assert_eq!(
            verified_audit_trail(app.db(), o1).await,
            vec![
                AuditEntry {
                    actor_user_id: Some(admin),
                    action: "member_added".into(),
                    subject_type: "user".into(),
                    subject_id: user,
                    details: json!({ "role": "user" }),
                },
                AuditEntry {
                    actor_user_id: Some(admin),
                    action: "relationship_created".into(),
                    subject_type: "coaching_relationship".into(),
                    subject_id: relationship.id,
                    details: json!({ "kind": "colleague", "coach_id": coach, "coachee_id": user }),
                },
            ]
        );
    })
    .await;
}

/// ORG-46
#[tokio::test]
async fn a_refused_add_records_nothing() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let o1 = app.fixture.o1;
        // Not an O1 member, so the relationship is refused after the member is added.
        let stranger = seed_user(app.db()).await;

        let reply = app
            .send(
                Caller::Admin,
                Method::POST,
                &format!("/organizations/{o1}/users"),
                Some(new_member_body(stranger)),
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.body);
        assert_eq!(user_with_email(app.db(), NEW_EMAIL).await, None);
        assert_eq!(verified_audit_trail(app.db(), o1).await, vec![]);
    })
    .await;
}

/// ORG-52
#[tokio::test]
async fn deleting_a_member_records_each_relationship_it_deletes() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, coach) = (app.fixture.o1, app.fixture.user(Caller::Coach));
        let (admin, member) = (
            app.fixture.user(Caller::Admin),
            app.fixture.user(Caller::Member),
        );
        let created = app
            .send(
                Caller::Admin,
                Method::POST,
                &format!("/organizations/{o1}/coaching_relationships"),
                Some(json!({ "coach_id": coach, "coachee_id": member, "kind": "colleague" })),
            )
            .await;
        assert_eq!(created.status, StatusCode::OK, "{}", created.body);
        let recorded_before = verified_audit_trail(app.db(), o1).await;

        let reply = app
            .send(
                Caller::Admin,
                Method::DELETE,
                &format!("/organizations/{o1}/users/{member}"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(
            verified_audit_trail(app.db(), o1).await[recorded_before.len()..],
            [AuditEntry {
                actor_user_id: Some(admin),
                action: "relationship_deleted".into(),
                subject_type: "coaching_relationship".into(),
                subject_id: created_id(&created),
                details: json!({ "kind": "colleague", "coach_id": coach, "coachee_id": member }),
            }, AuditEntry {
                actor_user_id: Some(admin),
                action: "member_removed".into(),
                subject_type: "user".into(),
                subject_id: member,
                details: json!({}),
            }]
        );
    })
    .await;
}

/// ORG-53
#[tokio::test]
async fn deleting_a_member_records_their_removal() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, admin, member) = (
            app.fixture.o1,
            app.fixture.user(Caller::Admin),
            app.fixture.user(Caller::Member),
        );

        let reply = app
            .send(
                Caller::Admin,
                Method::DELETE,
                &format!("/organizations/{o1}/users/{member}"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(
            verified_audit_trail(app.db(), o1).await,
            vec![AuditEntry {
                actor_user_id: Some(admin),
                action: "member_removed".into(),
                subject_type: "user".into(),
                subject_id: member,
                details: json!({}),
            }]
        );
    })
    .await;
}
