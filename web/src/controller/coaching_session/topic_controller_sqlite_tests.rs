//! Who can list, add, change, reorder, defer, delete and restore a session's topics.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::access_world::{session_topics, topic_row, TopicRow};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::topic_status::Status;
use domain::Id;
use serde_json::json;

use crate::test_utils::sqlite::{ids, TestApp, NON_PARTICIPANTS};

/// The path of `topic` under `session`, plus `suffix`.
fn topic_path(session: Id, topic: Id, suffix: &str) -> String {
    format!("/coaching_sessions/{session}/topics/{topic}{suffix}")
}

/// The topic as stored; it must exist.
async fn topic(app: &TestApp, topic: Id) -> TopicRow {
    topic_row(app.db(), topic).await.expect("the topic exists")
}

/// Asserts every non-participant is refused `method` on `path` and T1 and T2 are unchanged.
async fn assert_non_participants_refused(method: Method, path: impl Fn(&TestApp) -> String) {
    let mut app = TestApp::new().await;
    let (s1, t1, t2) = (app.world.s1, app.world.t1, app.world.t2);
    let path = path(&app);
    let topics = session_topics(app.db(), s1).await;
    let before = (topic(&app, t1).await, topic(&app, t2).await);

    for caller in NON_PARTICIPANTS {
        let (status, _) = app
            .expect(caller, method.clone(), &path, Some(json!({})))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
    }
    assert_eq!(session_topics(app.db(), s1).await, topics);
    assert_eq!((topic(&app, t1).await, topic(&app, t2).await), before);
}

/// TOP-06 and TOP-07: either participant edits the other's topic.
async fn assert_participant_edits(caller: Caller, topic_of: impl Fn(&TestApp) -> Id) {
    let mut app = TestApp::new().await;
    let s1 = app.world.s1;
    let target = topic_of(&app);

    let reply = app
        .send(
            caller,
            Method::PUT,
            &topic_path(s1, target, ""),
            Some(json!({ "body": "Edited kumquat" })),
        )
        .await;

    assert_eq!((reply.status, reply.ids()), (StatusCode::OK, ids([target])));
    assert_eq!(topic(&app, target).await.body, "Edited kumquat");
}

/// TOP-09 and TOP-10: `caller` deletes T2.
async fn assert_deletes_t2(caller: Caller) {
    let mut app = TestApp::new().await;
    let (s1, t2) = (app.world.s1, app.world.t2);

    let reply = app
        .send(caller, Method::DELETE, &topic_path(s1, t2, ""), None)
        .await;

    assert_eq!((reply.status, reply.ids()), (StatusCode::OK, ids([t2])));
    assert!(topic(&app, t2).await.deleted);
}

/// Deletes T2 as `caller`, for the undo tests.
async fn delete_t2(app: &mut TestApp, caller: Caller) {
    let (s1, t2) = (app.world.s1, app.world.t2);
    let (status, _) = app
        .expect(caller, Method::DELETE, &topic_path(s1, t2, ""), None)
        .await;
    assert_eq!(status, StatusCode::OK, "{caller:?} deletes T2");
}

/// TOP-01
#[tokio::test]
async fn participants_list_the_sessions_topics() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let world = &app.world;
        let path = format!("/coaching_sessions/{}/topics", world.s1);
        let expected = ids([world.t1, world.t2]);

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

/// TOP-02
#[tokio::test]
async fn non_participants_cannot_list_the_sessions_topics() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/coaching_sessions/{}/topics", app.world.s1);

        for caller in NON_PARTICIPANTS {
            let (status, _) = app.expect(caller, Method::GET, &path, None).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
    })
    .await;
}

/// TOP-03
#[tokio::test]
async fn participants_add_a_topic_authored_by_themselves() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let s1 = app.world.s1;
            let before = session_topics(app.db(), s1).await;

            let reply = app
                .send(
                    caller,
                    Method::POST,
                    &format!("/coaching_sessions/{s1}/topics"),
                    Some(json!({ "body": "New topic kumquat" })),
                )
                .await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let created: Vec<Id> = reply.ids().into_iter().collect();
            let [new] = created[..] else {
                panic!("{caller:?}: one new topic id, got {created:?}");
            };
            assert!(!before.contains(&new), "{caller:?}");
            let stored = topic(&app, new).await;
            assert_eq!(
                (stored.session, stored.author),
                (s1, app.world.user(caller)),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// TOP-04
#[tokio::test]
async fn non_participants_cannot_add_a_topic() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.world.s1;
        let before = session_topics(app.db(), s1).await;

        for caller in NON_PARTICIPANTS {
            let (status, _) = app
                .expect(
                    caller,
                    Method::POST,
                    &format!("/coaching_sessions/{s1}/topics"),
                    Some(json!({ "body": "New topic kumquat" })),
                )
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        assert_eq!(session_topics(app.db(), s1).await, before);
    })
    .await;
}

/// TOP-05
#[tokio::test]
async fn participants_reorder_the_sessions_topics() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let (s1, t1, t2) = (app.world.s1, app.world.t1, app.world.t2);

            let reply = app
                .send(
                    caller,
                    Method::PATCH,
                    &format!("/coaching_sessions/{s1}/topics/reorder"),
                    Some(json!({ "ordered_ids": [t2, t1] })),
                )
                .await;

            assert_eq!(
                (reply.status, reply.ids()),
                (StatusCode::OK, ids([t1, t2])),
                "{caller:?}"
            );
            assert!(
                topic(&app, t2).await.display_order < topic(&app, t1).await.display_order,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// TOP-06
#[tokio::test]
async fn a_coach_edits_the_coachees_topic() {
    within_time_limit(assert_participant_edits(Caller::Coach, |b| b.world.t2)).await;
}

/// TOP-07
#[tokio::test]
async fn a_coachee_edits_the_coachs_topic() {
    within_time_limit(assert_participant_edits(Caller::Coachee, |b| b.world.t1)).await;
}

/// TOP-08
#[tokio::test]
async fn non_participants_cannot_edit_a_topic() {
    within_time_limit(async {
        assert_non_participants_refused(Method::PUT, |b| topic_path(b.world.s1, b.world.t1, ""))
            .await;

        // The matrix's note: a topic outside the session in the path is not found.
        let mut app = TestApp::new().await;
        let (s3, t1) = (app.world.s3, app.world.t1);
        let before = topic(&app, t1).await;
        let (status, _) = app
            .expect(
                Caller::Coach,
                Method::PUT,
                &topic_path(s3, t1, ""),
                Some(json!({ "body": "Edited kumquat" })),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(topic(&app, t1).await, before);
    })
    .await;
}

/// TOP-09
#[tokio::test]
async fn a_coach_deletes_the_coachees_topic() {
    within_time_limit(assert_deletes_t2(Caller::Coach)).await;
}

/// TOP-10
#[tokio::test]
async fn a_coachee_deletes_their_own_topic() {
    within_time_limit(assert_deletes_t2(Caller::Coachee)).await;
}

/// TOP-11
#[tokio::test]
async fn a_coachee_cannot_delete_the_coachs_topic() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, t1) = (app.world.s1, app.world.t1);
        let before = topic(&app, t1).await;

        let (status, _) = app
            .expect(
                Caller::Coachee,
                Method::DELETE,
                &topic_path(s1, t1, ""),
                None,
            )
            .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(topic(&app, t1).await, before);
    })
    .await;
}

/// TOP-12
#[tokio::test]
async fn non_participants_cannot_delete_a_topic() {
    within_time_limit(assert_non_participants_refused(Method::DELETE, |b| {
        topic_path(b.world.s1, b.world.t1, "")
    }))
    .await;
}

/// TOP-16
#[tokio::test]
async fn participants_set_a_topics_status() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let (s1, t1) = (app.world.s1, app.world.t1);

            let reply = app
                .send(
                    caller,
                    Method::PATCH,
                    &topic_path(s1, t1, "/status"),
                    Some(json!({ "status": "Discussed" })),
                )
                .await;

            assert_eq!(
                (reply.status, reply.ids()),
                (StatusCode::OK, ids([t1])),
                "{caller:?}"
            );
            assert_eq!(
                topic(&app, t1).await.status,
                Status::Discussed,
                "{caller:?}"
            );
        }
    })
    .await;
}

/// TOP-17
#[tokio::test]
async fn non_participants_cannot_set_a_topics_status() {
    within_time_limit(assert_non_participants_refused(Method::PATCH, |b| {
        topic_path(b.world.s1, b.world.t1, "/status")
    }))
    .await;
}

/// TOP-18
#[tokio::test]
async fn only_the_author_undoes_a_topics_delete() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, t2) = (app.world.s1, app.world.t2);
        delete_t2(&mut app, Caller::Coach).await;

        let (status, _) = app
            .expect(
                Caller::Coach,
                Method::POST,
                &topic_path(s1, t2, "/undo"),
                None,
            )
            .await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(topic(&app, t2).await.deleted);
    })
    .await;
}

/// TOP-19
#[tokio::test]
async fn the_author_undoes_a_topics_delete() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let (s1, t2) = (app.world.s1, app.world.t2);
        delete_t2(&mut app, Caller::Coach).await;

        let reply = app
            .send(
                Caller::Coachee,
                Method::POST,
                &topic_path(s1, t2, "/undo"),
                None,
            )
            .await;

        assert_eq!((reply.status, reply.ids()), (StatusCode::OK, ids([t2])));
        let restored = topic(&app, t2).await;
        assert_eq!((restored.session, restored.deleted), (s1, false));
    })
    .await;
}

/// TOP-20
#[tokio::test]
async fn participants_undo_a_topics_defer() {
    within_time_limit(async {
        for caller in [Caller::Coach, Caller::Coachee] {
            let mut app = TestApp::new().await;
            let (s1, t1) = (app.world.s1, app.world.t1);
            let (status, _) = app
                .expect(
                    Caller::Coach,
                    Method::PATCH,
                    &topic_path(s1, t1, "/status"),
                    Some(json!({ "status": "Deferred" })),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{caller:?}: T1 is deferred");
            let destination = topic(&app, t1).await.session;
            assert_ne!(destination, s1, "{caller:?}: T1 moved to the next session");

            let reply = app
                .send(
                    caller,
                    Method::POST,
                    &topic_path(destination, t1, "/undo"),
                    None,
                )
                .await;

            assert_eq!(
                (reply.status, reply.ids()),
                (StatusCode::OK, ids([t1])),
                "{caller:?}"
            );
            assert_eq!(topic(&app, t1).await.session, s1, "{caller:?}");
        }
    })
    .await;
}

/// TOP-21
#[tokio::test]
async fn non_participants_cannot_undo_a_topic_change() {
    within_time_limit(assert_non_participants_refused(Method::POST, |b| {
        topic_path(b.world.s1, b.world.t1, "/undo")
    }))
    .await;
}
