//! Which sessions appear in a person's own session list, and who may ask for it.
//!
//! Each test signs in as one kind of person, asks for a list against a real database, and
//! checks that it holds exactly the right records, or that the request is refused.
//! How the routes behave once allowed is tested in `coaching_session_controller_tests.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use std::collections::{BTreeMap, BTreeSet};

use axum::http::{Method, StatusCode};
use chrono::Utc;
use domain::test_utils::sqlite::fixture::make_colleagues;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;
use serde_json::{json, Value};

use crate::test_utils::sqlite::{ids, seed_super_coaching, TestApp};

/// Asserts `caller`'s own session list is `expected`.
async fn assert_own_sessions(caller: Caller, expected: impl Fn(&TestApp) -> BTreeSet<Id>) {
    let mut app = TestApp::new().await;
    let user = app.fixture.user(caller);
    let expected = expected(&app);

    assert_eq!(
        app.expect(
            caller,
            Method::GET,
            &format!("/users/{user}/coaching_sessions"),
            None
        )
        .await,
        (StatusCode::OK, expected),
        "{caller:?}"
    );
}

/// SES-27
#[tokio::test]
async fn a_coach_lists_their_own_sessions() {
    within_time_limit(assert_own_sessions(Caller::Coach, |b| {
        let fixture = &b.fixture;
        ids([
            fixture.s1,
            fixture.s3,
            fixture.sr1_sessions[0],
            fixture.sr1_sessions[1],
            fixture.sr3_sessions[0],
            fixture.sr3_sessions[1],
        ])
    }))
    .await;
}

/// SES-28
#[tokio::test]
async fn a_coachee_lists_their_own_sessions() {
    within_time_limit(assert_own_sessions(Caller::Coachee, |b| {
        let fixture = &b.fixture;
        ids([
            fixture.s1,
            fixture.s2,
            fixture.sr1_sessions[0],
            fixture.sr1_sessions[1],
        ])
    }))
    .await;
}

/// SES-29
#[tokio::test]
async fn only_the_user_lists_their_own_sessions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coachee = app.fixture.user(Caller::Coachee);

        for suffix in ["", "/counts", "/nearest"] {
            let path = format!("/users/{coachee}/coaching_sessions{suffix}");
            for caller in [Caller::Coach, Caller::Member, Caller::Admin, Caller::Super] {
                let (status, _) = app.expect(caller, Method::GET, &path, None).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {suffix}");
            }
        }
    })
    .await;
}

/// SES-30
#[tokio::test]
async fn a_removed_coachee_lists_none_of_their_own_sessions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let removed = app.fixture.user(Caller::Removed);

        assert_eq!(
            app.expect(
                Caller::Removed,
                Method::GET,
                &format!("/users/{removed}/coaching_sessions"),
                None
            )
            .await,
            (StatusCode::OK, BTreeSet::new())
        );
    })
    .await;
}

/// SES-32
#[tokio::test]
async fn a_coachee_reads_the_nearest_session_of_each_relationship() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let coachee = fixture.user(Caller::Coachee);
        let (r1, r2, r3) = (fixture.r1, fixture.r2, fixture.r3);
        let expected = ids([fixture.s1, fixture.s2]);
        let around = Utc::now().format("%Y-%m-%dT%H:%M:%S");

        // R3 is not the coachee's, so it adds nothing.
        for relationships in [format!("{r1},{r2}"), format!("{r1},{r2},{r3}")] {
            let path = format!(
                "/users/{coachee}/coaching_sessions/nearest\
                 ?coaching_relationship_ids={relationships}&around={around}&before=1&after=1"
            );
            assert_eq!(
                app.expect(Caller::Coachee, Method::GET, &path, None).await,
                (StatusCode::OK, expected.clone()),
                "{relationships}"
            );
        }
    })
    .await;
}

/// SES-43
#[tokio::test]
async fn a_removed_coachee_reads_no_nearest_session() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (removed, r3) = (app.fixture.user(Caller::Removed), app.fixture.r3);
        let around = Utc::now().format("%Y-%m-%dT%H:%M:%S");
        // S3 and SR3's sessions lie after `around` in R3.
        let path = format!(
            "/users/{removed}/coaching_sessions/nearest\
             ?coaching_relationship_ids={r3}&around={around}&before=1&after=1"
        );

        assert_eq!(
            app.expect(Caller::Removed, Method::GET, &path, None).await,
            (StatusCode::OK, BTreeSet::new())
        );
    })
    .await;
}

/// USR-01
#[tokio::test]
async fn a_coachees_sessions_include_their_relationships() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let fixture = &app.fixture;
        let coachee = fixture.user(Caller::Coachee);
        let expected = ids([
            fixture.s1,
            fixture.s2,
            fixture.sr1_sessions[0],
            fixture.sr1_sessions[1],
        ]);
        let path = format!("/users/{coachee}/coaching_sessions?include=relationship");

        let reply = app.send(Caller::Coachee, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.ids(), expected);
        for session in reply.body["data"].as_array().expect("a list of sessions") {
            assert!(session["relationship"].is_object(), "{session}");
            assert!(session["participants"].is_array(), "{session}");
            assert_eq!(
                session["participants"][1]["id"],
                coachee.to_string(),
                "{session}"
            );
        }
    })
    .await;
}

/// SES-44
#[tokio::test]
async fn a_super_admin_lists_their_own_session_in_an_org_they_hold_no_role_in() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let super_admin = app.fixture.user(Caller::Super);
        let expected = ids([seed_super_coaching(&app).await.session]);
        let path = format!("/users/{super_admin}/coaching_sessions");

        assert_eq!(
            app.expect(Caller::Super, Method::GET, &path, None).await,
            (StatusCode::OK, expected)
        );
    })
    .await;
}

/// SES-45
#[tokio::test]
async fn a_super_admin_reads_their_nearest_session_in_an_org_they_hold_no_role_in() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let super_admin = app.fixture.user(Caller::Super);
        let seeded = seed_super_coaching(&app).await;
        let around = Utc::now().format("%Y-%m-%dT%H:%M:%S");
        // The session lies after `around`.
        let path = format!(
            "/users/{super_admin}/coaching_sessions/nearest\
             ?coaching_relationship_ids={}&around={around}&before=1&after=1",
            seeded.relationship
        );

        assert_eq!(
            app.expect(Caller::Super, Method::GET, &path, None).await,
            (StatusCode::OK, ids([seeded.session]))
        );
    })
    .await;
}

/// KIND-23
#[tokio::test]
async fn an_included_relationship_tells_the_caller_what_they_may_do() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coach = app.fixture.user(Caller::Coach);
        let path = format!("/users/{coach}/coaching_sessions?include=relationship");

        let reply = app.send(Caller::Coach, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK);
        let sessions = reply.body["data"].as_array().expect("a list of sessions");
        assert!(!sessions.is_empty(), "{}", reply.body);
        for session in sessions {
            assert_eq!(
                session["relationship"]["viewer"],
                json!({ "role": "coach", "can_schedule_sessions": true, "sees_all_actions": true }),
                "{session}"
            );
            assert_eq!(
                session["relationship"]["suggests_actions"], true,
                "{session}"
            );
        }
    })
    .await;
}

/// `caller`'s own session list with `query`, as its sessions keyed by id.
async fn own_sessions(app: &mut TestApp, caller: Caller, query: &str) -> BTreeMap<Id, Value> {
    let user = app.fixture.user(caller);
    let path = format!("/users/{user}/coaching_sessions{query}");
    let reply = app.send(caller, Method::GET, &path, None).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
    reply.body["data"]
        .as_array()
        .expect("a list of sessions")
        .iter()
        .map(|session| {
            let id = session["id"]
                .as_str()
                .and_then(|id| id.parse().ok())
                .expect("each session has an id");
            (id, session.clone())
        })
        .collect()
}

/// KIND-49
#[tokio::test]
async fn an_included_relationship_lists_the_sessions_coach_then_its_coachee() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, coach, coachee) = (
            app.fixture.s1,
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Coachee),
        );

        let sessions = own_sessions(&mut app, Caller::Coach, "?include=relationship").await;

        let session = &sessions[&s1];
        assert_eq!(
            session["participants"],
            json!([
                {
                    "id": coach,
                    "first_name": "Coach",
                    "last_name": "Access",
                    "email": Caller::Coach.email(),
                    "role": "coach",
                    "is_member": true,
                },
                {
                    "id": coachee,
                    "first_name": "Coachee",
                    "last_name": "Access",
                    "email": Caller::Coachee.email(),
                    "role": "coachee",
                    "is_member": true,
                },
            ]),
            "{session}"
        );
        for key in ["coach", "coachee"] {
            assert!(session.get(key).is_none(), "{key}: {session}");
        }
        for key in ["coach_is_member", "coachee_is_member"] {
            assert!(
                session["relationship"].get(key).is_none(),
                "{key}: {session}"
            );
        }
    })
    .await;
}

/// KIND-49, KIND-23
#[tokio::test]
async fn a_colleague_sessions_nearest_read_lists_both_as_colleagues_coach_seat_first() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (r1, s1, coach, coachee) = (
            app.fixture.r1,
            app.fixture.s1,
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Coachee),
        );
        make_colleagues(app.db(), r1).await;
        let around = Utc::now().format("%Y-%m-%dT%H:%M:%S");
        let path = format!(
            "/users/{coachee}/coaching_sessions/nearest\
             ?coaching_relationship_ids={r1}&around={around}&before=1&after=1\
             &include=relationship"
        );

        let reply = app.send(Caller::Coachee, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(reply.ids(), ids([s1]));
        let session = &reply.body["data"][0];
        let participants = &session["participants"];
        assert_eq!(
            (&participants[0]["id"], &participants[0]["role"]),
            (&json!(coach), &json!("colleague")),
            "{session}"
        );
        assert_eq!(
            (&participants[1]["id"], &participants[1]["role"]),
            (&json!(coachee), &json!("colleague")),
            "{session}"
        );
        assert_eq!(
            session["relationship"]["viewer"],
            json!({ "role": "colleague", "can_schedule_sessions": true, "sees_all_actions": true }),
            "{session}"
        );
    })
    .await;
}

/// KIND-49
#[tokio::test]
async fn a_coachs_session_list_marks_a_coachee_who_left() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s3, coach, removed) = (
            app.fixture.s3,
            app.fixture.user(Caller::Coach),
            app.fixture.user(Caller::Removed),
        );

        let sessions = own_sessions(&mut app, Caller::Coach, "?include=relationship").await;

        let participants = &sessions[&s3]["participants"];
        assert_eq!(
            (&participants[0]["id"], &participants[0]["is_member"]),
            (&json!(coach), &json!(true)),
            "{participants}"
        );
        assert_eq!(
            (&participants[1]["id"], &participants[1]["is_member"]),
            (&json!(removed), &json!(false)),
            "{participants}"
        );
    })
    .await;
}

/// KIND-49
#[tokio::test]
async fn a_session_list_without_its_relationship_lists_no_participants() {
    within_time_limit(async {
        let mut app = TestApp::new().await;

        let sessions = own_sessions(&mut app, Caller::Coach, "").await;

        assert!(!sessions.is_empty());
        for session in sessions.values() {
            assert!(session.get("participants").is_none(), "{session}");
        }
    })
    .await;
}
