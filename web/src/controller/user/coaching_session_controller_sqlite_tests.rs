//! A user's own sessions (`SES`, `USR`) through `/users/{id}/coaching_sessions`, on SQLite as
//! each access-matrix caller.

use std::collections::BTreeSet;

use axum::http::{Method, StatusCode};
use chrono::Utc;
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;

use crate::test_utils::sqlite::{ids, TestApp};

/// Asserts `caller`'s own session list is `expected`.
async fn assert_own_sessions(caller: Caller, expected: impl Fn(&TestApp) -> BTreeSet<Id>) {
    let mut app = TestApp::new().await;
    let user = app.world.user(caller);
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
        let world = &b.world;
        ids([
            world.s1,
            world.s3,
            world.sr1_sessions[0],
            world.sr1_sessions[1],
            world.sr3_sessions[0],
            world.sr3_sessions[1],
        ])
    }))
    .await;
}

/// SES-28
#[tokio::test]
async fn a_coachee_lists_their_own_sessions() {
    within_time_limit(assert_own_sessions(Caller::Coachee, |b| {
        let world = &b.world;
        ids([
            world.s1,
            world.s2,
            world.sr1_sessions[0],
            world.sr1_sessions[1],
        ])
    }))
    .await;
}

/// SES-29
#[tokio::test]
async fn only_the_user_lists_their_own_sessions() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let coachee = app.world.user(Caller::Coachee);

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
        let removed = app.world.user(Caller::Removed);

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
        let world = &app.world;
        let coachee = world.user(Caller::Coachee);
        let (r1, r2, r3) = (world.r1, world.r2, world.r3);
        let expected = ids([world.s1, world.s2]);
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

/// USR-01
#[tokio::test]
async fn a_coachees_sessions_include_their_relationships() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let world = &app.world;
        let coachee = world.user(Caller::Coachee);
        let expected = ids([
            world.s1,
            world.s2,
            world.sr1_sessions[0],
            world.sr1_sessions[1],
        ]);
        let path = format!("/users/{coachee}/coaching_sessions?include=relationship");

        let reply = app.send(Caller::Coachee, Method::GET, &path, None).await;

        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.ids(), expected);
        for session in reply.body["data"].as_array().expect("a list of sessions") {
            for field in ["relationship", "coach", "coachee"] {
                assert!(session[field].is_object(), "{field} in {session}");
            }
            assert_eq!(session["coachee"]["id"], coachee.to_string(), "{session}");
        }
    })
    .await;
}
