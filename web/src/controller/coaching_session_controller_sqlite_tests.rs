//! Who can see, schedule, change and delete coaching sessions, and who a resent invite reaches.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! How the routes behave once allowed is tested in the tests in `coaching_session_controller.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use chrono::{TimeDelta, Utc};
use domain::test_utils::sqlite::fixture::{seed_content_free_session, set_google_meeting_url};
use domain::test_utils::sqlite::read_back::{
    relationship_sessions, session_content, session_ical_sequence, session_meeting_url,
    session_relationship, session_schedule, session_title, session_viewers,
};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use crate::test_utils::sqlite::{
    assert_removed_coach_resend_refused, assert_resend_mails_both, ids, TestApp, NON_PARTICIPANTS,
};

/// S1's invite resend path.
fn session_resend(app: &TestApp) -> String {
    format!("/coaching_sessions/{}/resend_invite", app.fixture.s1)
}

/// R1's Google Meet link, which a new R1 session reuses.
const R1_LINK: &str = "https://meet.google.com/kum-quat-r1";

/// A session create body for `relationship`, five days out, asking for a Google link.
fn create_with_provider(relationship: Id) -> Value {
    let date = (Utc::now() + TimeDelta::days(5))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    json!({ "coaching_relationship_id": relationship, "date": date, "provider": "google" })
}

/// A new title for S1 or S3.
const NEW_TITLE: &str = "Retitled kumquat";

/// A session create body for `relationship_id`, five days out.
fn create_body(relationship_id: Id) -> Value {
    let date = (Utc::now() + TimeDelta::days(5))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    json!({ "coaching_relationship_id": relationship_id, "date": date })
}

/// Asserts each caller's session create in `relationship` is refused with `expected`, with
/// no session written, no document created and no email sent.
async fn assert_create_refused(
    callers: &[Caller],
    relationship: impl Fn(&TestApp) -> Id,
    expected: StatusCode,
) {
    let mut app = TestApp::new().await;
    let relationship = relationship(&app);
    let before = relationship_sessions(app.db(), relationship).await;
    let documents = app.expect_tiptap("POST", 0).await;
    let emails = app.expect_emails(0).await;

    for &caller in callers {
        let reply = app
            .send(
                caller,
                Method::POST,
                "/coaching_sessions",
                Some(create_body(relationship)),
            )
            .await;
        assert_eq!(reply.status, expected, "{caller:?}");
        if expected == StatusCode::CONFLICT {
            assert_eq!(
                reply.body["details"]["reason"], "coachee_not_a_member",
                "{caller:?}"
            );
        }
        assert_eq!(
            relationship_sessions(app.db(), relationship).await,
            before,
            "{caller:?}"
        );
    }
    documents.assert_async().await;
    emails.assert_async().await;
}

/// Asserts each caller's `method` on S1 at `suffix` with `body` is refused, leaving S1's
/// schedule and title as they were.
async fn assert_s1_write_refused(callers: &[Caller], method: Method, suffix: &str, body: Value) {
    let mut app = TestApp::new().await;
    let s1 = app.fixture.s1;
    let path = format!("/coaching_sessions/{s1}{suffix}");
    let schedule = session_schedule(app.db(), s1).await;
    let title = session_title(app.db(), s1).await;

    for &caller in callers {
        let (status, _) = app
            .expect(caller, method.clone(), &path, Some(body.clone()))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        assert_eq!(session_schedule(app.db(), s1).await, schedule, "{caller:?}");
        assert_eq!(session_title(app.db(), s1).await, title, "{caller:?}");
    }
}

/// A PUT body keeping the session's date and changing its title.
async fn retitle_body(app: &TestApp, session: Id) -> Value {
    let (date, _) = session_schedule(app.db(), session)
        .await
        .expect("the session is seeded");
    json!({ "date": date, "title": NEW_TITLE })
}

/// Asserts each caller's DELETE of a fresh content-free R1 session is refused, leaving it
/// and its document in place.
async fn assert_delete_refused(callers: &[Caller]) {
    let mut app = TestApp::new().await;
    let s4 = seed_content_free_session(app.db(), app.fixture.r1).await;
    let schedule = session_schedule(app.db(), s4).await;
    let documents_deleted = app.expect_tiptap("DELETE", 0).await;

    for &caller in callers {
        let (status, _) = app
            .expect(
                caller,
                Method::DELETE,
                &format!("/coaching_sessions/{s4}"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        assert!(schedule.is_some(), "{caller:?}");
        assert_eq!(session_schedule(app.db(), s4).await, schedule, "{caller:?}");
    }
    documents_deleted.assert_async().await;
}

/// Asserts each caller's resend of `session`'s invite is answered `expected`, with no email
/// sent and the calendar SEQUENCE unchanged.
async fn assert_resend_refused(
    callers: &[Caller],
    session: impl Fn(&TestApp) -> Id,
    expected: StatusCode,
) {
    let mut app = TestApp::new().await;
    let session = session(&app);
    let sequence = session_ical_sequence(app.db(), session).await;
    let emails = app.expect_emails(0).await;

    for &caller in callers {
        let reply = app
            .send(
                caller,
                Method::POST,
                &format!("/coaching_sessions/{session}/resend_invite"),
                None,
            )
            .await;
        assert_eq!(reply.status, expected, "{caller:?}");
        if expected == StatusCode::CONFLICT {
            assert_eq!(
                reply.body["details"]["reason"], "coachee_not_a_member",
                "{caller:?}"
            );
        }
        assert_eq!(
            session_ical_sequence(app.db(), session).await,
            sequence,
            "{caller:?}"
        );
    }
    emails.assert_async().await;
}

/// Wide enough to hold every seeded session.
const DATE_RANGE: &str = "from_date=2000-01-01&to_date=2100-01-01";

/// MAIL-04
#[tokio::test]
async fn a_session_resend_mails_both_members() {
    within_time_limit(assert_resend_mails_both(session_resend)).await;
}

/// MAIL-04
#[tokio::test]
async fn a_coach_who_left_cannot_resend_a_sessions_invite() {
    within_time_limit(assert_removed_coach_resend_refused(session_resend)).await;
}

/// MEET-02
#[tokio::test]
async fn only_the_coachs_create_attaches_a_meeting_link() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, r3, s1) = (app.fixture.r1, app.fixture.r3, app.fixture.s1);
        set_google_meeting_url(app.db(), s1, R1_LINK).await;

        // SES-10 to SES-14: refused, with no session, document or link written.
        let documents = app.expect_tiptap("POST", 0).await;
        for (caller, relationship) in [
            (Caller::Coachee, r1),
            (Caller::Member, r1),
            (Caller::OtherCoach, r1),
            (Caller::Admin, r1),
            (Caller::Super, r1),
            (Caller::Outsider, r1),
            (Caller::Removed, r3),
        ] {
            let before = relationship_sessions(app.db(), relationship).await;
            let (status, _) = app
                .expect(
                    caller,
                    Method::POST,
                    "/coaching_sessions",
                    Some(create_with_provider(relationship)),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(
                relationship_sessions(app.db(), relationship).await,
                before,
                "{caller:?}"
            );
        }
        documents.assert_async().await;

        // SES-09: the coach's new session reuses R1's link.
        let reply = app
            .send(
                Caller::Coach,
                Method::POST,
                "/coaching_sessions",
                Some(create_with_provider(r1)),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let created: Vec<Id> = reply.ids().into_iter().collect();
        let [session] = created[..] else {
            panic!("one new session id, got {created:?}");
        };
        assert_eq!(
            session_meeting_url(app.db(), session).await,
            Some(Some(R1_LINK.to_string()))
        );
    })
    .await;
}

/// MEET-03
#[tokio::test]
async fn a_coach_sets_the_sessions_meeting_link() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;
        let (date, _) = session_schedule(app.db(), s1).await.expect("S1 is seeded");

        let reply = app
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/coaching_sessions/{s1}"),
                Some(json!({ "date": date, "meeting_url": R1_LINK, "provider": "google" })),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 204);
        assert_eq!(
            session_meeting_url(app.db(), s1).await,
            Some(Some(R1_LINK.to_string()))
        );
    })
    .await;
}

/// SES-01
#[tokio::test]
async fn participants_list_the_relationships_sessions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let path = format!(
            "/coaching_sessions?coaching_relationship_id={}&{DATE_RANGE}",
            fixture.r1
        );
        let expected = ids([fixture.s1, fixture.sr1_sessions[0], fixture.sr1_sessions[1]]);

        for caller in [Caller::Coach, Caller::Coachee] {
            assert_eq!(
                app.expect(caller, Method::GET, &path, None).await,
                (StatusCode::OK, expected.clone()),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SES-02
#[tokio::test]
async fn non_participants_cannot_list_the_relationships_sessions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!(
            "/coaching_sessions?coaching_relationship_id={}&{DATE_RANGE}",
            app.fixture.r1
        );

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// SES-03
#[tokio::test]
async fn a_removed_coachee_cannot_list_their_relationships_sessions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!(
            "/coaching_sessions?coaching_relationship_id={}&{DATE_RANGE}",
            app.fixture.r3
        );

        let (status, _) = app.expect(Caller::Removed, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// SES-04
#[tokio::test]
async fn participants_read_a_session_with_its_organization() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (o1, s1) = (app.fixture.o1, app.fixture.s1);

        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = app
                .send(
                    caller,
                    Method::GET,
                    &format!("/coaching_sessions/{s1}"),
                    None,
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.ids(), ids([s1]), "{caller:?}");
            assert_eq!(
                reply.body["data"]["organization_id"],
                o1.to_string(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SES-05
#[tokio::test]
async fn non_participants_cannot_read_a_session() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/coaching_sessions/{}", app.fixture.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// SES-06
#[tokio::test]
async fn a_removed_coachee_cannot_read_their_session() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/coaching_sessions/{}", app.fixture.s3);

        let (status, _) = app.expect(Caller::Removed, Method::GET, &path, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await;
}

/// SES-08
#[tokio::test]
async fn non_participants_cannot_mark_a_session_viewed() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;
        let path = format!("/coaching_sessions/{s1}/view");

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::POST, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
            assert_eq!(
                session_viewers(app.db(), s1).await,
                BTreeSet::new(),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SES-09
#[tokio::test]
async fn a_coach_creates_a_session_with_its_notes_document() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let r1 = app.fixture.r1;
        let date = (Utc::now() + TimeDelta::days(5))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let documents_created = app.expect_tiptap("POST", 1).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::POST,
                "/coaching_sessions",
                Some(json!({ "coaching_relationship_id": r1, "date": date })),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 201);
        let created: Vec<Id> = reply.ids().into_iter().collect();
        let [session] = created[..] else {
            panic!("one new session id, got {created:?}");
        };
        assert_eq!(session_relationship(app.db(), session).await, Some(r1));
        documents_created.assert_async().await;
    })
    .await;
}

/// SES-10
#[tokio::test]
async fn a_coachee_cannot_create_a_session() {
    within_time_limit(assert_create_refused(
        &[Caller::Coachee],
        |b| b.fixture.r1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-11
#[tokio::test]
async fn members_outside_the_relationship_cannot_create_a_session() {
    within_time_limit(assert_create_refused(
        &[Caller::Member, Caller::OtherCoach],
        |b| b.fixture.r1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-12
#[tokio::test]
async fn admins_cannot_create_a_session() {
    within_time_limit(assert_create_refused(
        &[Caller::Admin, Caller::Super],
        |b| b.fixture.r1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-13
#[tokio::test]
async fn an_outsider_cannot_create_a_session() {
    within_time_limit(assert_create_refused(
        &[Caller::Outsider],
        |b| b.fixture.r1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-14
#[tokio::test]
async fn a_removed_coachee_cannot_create_a_session() {
    within_time_limit(assert_create_refused(
        &[Caller::Removed],
        |b| b.fixture.r3,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-15
#[tokio::test]
async fn a_coach_updates_a_session() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;
        let body = retitle_body(&app, s1).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/coaching_sessions/{s1}"),
                Some(body),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 204);
        assert_eq!(
            session_title(app.db(), s1).await,
            Some(Some(NEW_TITLE.to_string()))
        );
    })
    .await;
}

/// SES-16
#[tokio::test]
async fn a_coachee_cannot_update_a_session() {
    within_time_limit(async {
        let body = json!({ "date": "2100-01-01T00:00:00", "title": NEW_TITLE });
        assert_s1_write_refused(&[Caller::Coachee], Method::PUT, "", body).await;
    })
    .await;
}

/// SES-17
#[tokio::test]
async fn non_participants_cannot_update_a_session() {
    within_time_limit(async {
        let body = json!({ "date": "2100-01-01T00:00:00", "title": NEW_TITLE });
        assert_s1_write_refused(&NON_PARTICIPANTS, Method::PUT, "", body).await;
    })
    .await;
}

/// SES-18
#[tokio::test]
async fn a_coach_retitles_a_session_whose_coachee_left() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s3 = app.fixture.s3;
        let schedule = session_schedule(app.db(), s3).await;
        let body = retitle_body(&app, s3).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/coaching_sessions/{s3}"),
                Some(body),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 204);
        assert_eq!(
            session_title(app.db(), s3).await,
            Some(Some(NEW_TITLE.to_string()))
        );
        assert_eq!(session_schedule(app.db(), s3).await, schedule);
    })
    .await;
}

/// SES-19
#[tokio::test]
async fn either_participant_retitles_a_session() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;
        let path = format!("/coaching_sessions/{s1}/title");

        for caller in [Caller::Coach, Caller::Coachee] {
            let title = format!("{caller:?} kumquat");
            let reply = app
                .send(
                    caller,
                    Method::PATCH,
                    &path,
                    Some(json!({ "title": title })),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.ids(), ids([s1]), "{caller:?}");
            assert_eq!(reply.body["data"]["title"], title.as_str(), "{caller:?}");
            assert_eq!(
                session_title(app.db(), s1).await,
                Some(Some(title)),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// SES-20
#[tokio::test]
async fn non_participants_cannot_retitle_a_session() {
    within_time_limit(assert_s1_write_refused(
        &NON_PARTICIPANTS,
        Method::PATCH,
        "/title",
        json!({ "title": NEW_TITLE }),
    ))
    .await;
}

/// SES-21
#[tokio::test]
async fn a_coach_deletes_a_content_free_session() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s4 = seed_content_free_session(app.db(), app.fixture.r1).await;
        let document_deleted = app.expect_tiptap("DELETE", 1).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::DELETE,
                &format!("/coaching_sessions/{s4}"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 204);
        assert_eq!(session_schedule(app.db(), s4).await, None);
        document_deleted.assert_async().await;
    })
    .await;
}

/// SES-22
#[tokio::test]
async fn a_coachee_cannot_delete_a_session() {
    within_time_limit(assert_delete_refused(&[Caller::Coachee])).await;
}

/// SES-23
#[tokio::test]
async fn non_participants_cannot_delete_a_session() {
    within_time_limit(assert_delete_refused(&NON_PARTICIPANTS)).await;
}

/// SES-24
#[tokio::test]
async fn a_coach_resends_a_sessions_invite() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;
        let before = session_ical_sequence(app.db(), s1)
            .await
            .expect("S1 is seeded");
        let invites = app.expect_emails(2).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::POST,
                &format!("/coaching_sessions/{s1}/resend_invite"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::NO_CONTENT);
        assert_eq!(session_ical_sequence(app.db(), s1).await, Some(before + 1));
        invites.assert_async().await;
    })
    .await;
}

/// SES-25
#[tokio::test]
async fn a_coachee_cannot_resend_a_sessions_invite() {
    within_time_limit(assert_resend_refused(
        &[Caller::Coachee],
        |b| b.fixture.s1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-26
#[tokio::test]
async fn non_participants_cannot_resend_a_sessions_invite() {
    within_time_limit(assert_resend_refused(
        &NON_PARTICIPANTS,
        |b| b.fixture.s1,
        StatusCode::FORBIDDEN,
    ))
    .await;
}

/// SES-33
#[tokio::test]
async fn scheduling_with_a_coachee_who_left_is_refused() {
    within_time_limit(assert_create_refused(
        &[Caller::Coach],
        |b| b.fixture.r3,
        StatusCode::CONFLICT,
    ))
    .await;
}

/// SES-34
#[tokio::test]
async fn moving_a_session_whose_coachee_left_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s3 = app.fixture.s3;
        let before = session_schedule(app.db(), s3).await;
        let (date, _) = before.expect("S3 is seeded");
        let moved = (date + TimeDelta::days(1))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();

        let reply = app
            .send(
                Caller::Coach,
                Method::PUT,
                &format!("/coaching_sessions/{s3}"),
                Some(json!({ "date": moved })),
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_eq!(reply.body["details"]["reason"], "coachee_not_a_member");
        assert_eq!(session_schedule(app.db(), s3).await, before);
    })
    .await;
}

/// SES-35
#[tokio::test]
async fn resending_to_a_coachee_who_left_is_refused() {
    within_time_limit(assert_resend_refused(
        &[Caller::Coach],
        |b| b.fixture.s3,
        StatusCode::CONFLICT,
    ))
    .await;
}

/// SES-36
#[tokio::test]
async fn a_coach_deletes_a_session_whose_coachee_left() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s5 = seed_content_free_session(app.db(), app.fixture.r3).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::DELETE,
                &format!("/coaching_sessions/{s5}"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body["status_code"], 204);
        assert_eq!(session_schedule(app.db(), s5).await, None);
    })
    .await;
}

/// SES-37
#[tokio::test]
async fn deleting_a_session_with_content_is_refused() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let s1 = fixture.s1;
        let content: BTreeSet<Id> =
            ids([fixture.n1, fixture.ag1, fixture.a1, fixture.a2, fixture.a3]);
        let schedule = session_schedule(app.db(), s1).await;
        let documents_deleted = app.expect_tiptap("DELETE", 0).await;
        let emails = app.expect_emails(0).await;

        let reply = app
            .send(
                Caller::Coach,
                Method::DELETE,
                &format!("/coaching_sessions/{s1}"),
                None,
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT);
        assert_eq!(reply.body["details"]["reason"], "sessions_have_content");
        assert_eq!(reply.body["details"]["coaching_session_id"], s1.to_string());
        assert!(schedule.is_some());
        assert_eq!(session_schedule(app.db(), s1).await, schedule);
        assert_eq!(session_content(app.db(), s1).await, content);
        documents_deleted.assert_async().await;
        emails.assert_async().await;
    })
    .await;
}
