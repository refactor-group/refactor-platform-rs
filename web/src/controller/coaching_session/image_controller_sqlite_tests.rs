//! Who can upload, view, delete and restore images in a session's notes.
//!
//! Each test signs in as one kind of person, calls one of this module's routes against a
//! real database, and checks that the right people get through and everyone else is refused
//! with nothing changed.
//! How the routes behave once allowed is tested in `image_controller_tests.rs`.
//! `web/src/test_utils/README.md` describes the people and the database.

use axum::body::Body;
use axum::http::{Method, StatusCode};
use domain::test_utils::sqlite::fixture::IMG1_BYTES;
use domain::test_utils::sqlite::read_back::{image_deleted, session_images};
use domain::test_utils::sqlite::{within_time_limit, Caller};
use domain::Id;

use crate::test_utils::sqlite::{ids, Reply, TestApp, NON_PARTICIPANTS};

/// A valid 2x3 PNG, so the upload sees a real image.
const TINY_PNG: [u8; 73] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x03, 0x08, 0x02, 0x00, 0x00, 0x00, 0x36, 0x88, 0x49,
    0xd6, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x44, 0x0c, 0x28, 0x14, 0x00, 0x44, 0xd0, 0x05, 0xfb, 0xa4, 0xcf, 0xde, 0x80, 0x00, 0x00, 0x00,
    0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

const BOUNDARY: &str = "access-test-boundary";

/// A `multipart/form-data` body carrying TINY_PNG as the `file` part.
fn image_upload() -> Body {
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; \
         filename=\"paste.png\"\r\nContent-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&TINY_PNG);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    Body::from(body)
}

/// Uploads TINY_PNG to `session` as `caller`.
async fn upload(app: &mut TestApp, caller: Caller, session: Id) -> Reply {
    let content_type = format!("multipart/form-data; boundary={BOUNDARY}");
    app.send_with(
        caller,
        Method::POST,
        &format!("/coaching_sessions/{session}/images"),
        &[("content-type", content_type.as_str())],
        image_upload(),
    )
    .await
}

/// TRN-23
#[tokio::test]
async fn participants_upload_a_note_image() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;

        for caller in [Caller::Coach, Caller::Coachee] {
            let before = session_images(app.db(), s1).await;

            let reply = upload(&mut app, caller, s1).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(reply.body["status_code"], 201, "{caller:?}");
            let created: Vec<Id> = reply.ids().into_iter().collect();
            let [image] = created[..] else {
                panic!("{caller:?}: one new image id, got {created:?}");
            };
            assert!(!before.contains(&image), "{caller:?}");
            assert!(
                session_images(app.db(), s1).await.contains(&image),
                "{caller:?}"
            );
        }
    })
    .await;
}

/// TRN-24
#[tokio::test]
async fn non_participants_cannot_upload_a_note_image() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let s1 = app.fixture.s1;
        let before = session_images(app.db(), s1).await;

        for caller in NON_PARTICIPANTS {
            let reply = upload(&mut app, caller, s1).await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{caller:?}");
        }
        assert_eq!(session_images(app.db(), s1).await, before);
    })
    .await;
}

/// TRN-25
#[tokio::test]
async fn participants_read_a_note_image() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let path = format!("/coaching_session_images/{}", app.fixture.img1);

        // The local store cannot presign, so it serves the bytes rather than a redirect.
        for caller in [Caller::Coach, Caller::Coachee] {
            let reply = app.send(caller, Method::GET, &path, None).await;

            assert_eq!(reply.status, StatusCode::OK, "{caller:?}");
            assert_eq!(&reply.bytes[..], IMG1_BYTES, "{caller:?}");
        }
    })
    .await;
}

/// TRN-26
#[tokio::test]
async fn either_participant_deletes_and_restores_a_note_image() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let img1 = app.fixture.img1;

        // IMG1 was uploaded by COACH, so COACHEE acts on an image they did not upload.
        for caller in [Caller::Coach, Caller::Coachee] {
            for (method, suffix, deleted) in [
                (Method::DELETE, "", true),
                (Method::POST, "/restore", false),
            ] {
                let path = format!("/coaching_session_images/{img1}{suffix}");
                assert_eq!(
                    app.expect(caller, method.clone(), &path, None).await,
                    (StatusCode::OK, ids([img1])),
                    "{caller:?} {method}"
                );
                assert_eq!(
                    image_deleted(app.db(), img1).await,
                    Some(deleted),
                    "{caller:?} {method}"
                );
            }
        }
    })
    .await;
}

/// TRN-27
#[tokio::test]
async fn non_participants_cannot_read_delete_or_restore_a_note_image() {
    within_time_limit(async {
        let mut app = TestApp::new().await;
        let img1 = app.fixture.img1;
        let path = format!("/coaching_session_images/{img1}");

        for caller in NON_PARTICIPANTS {
            for method in [Method::GET, Method::DELETE] {
                let (status, _) = app.expect(caller, method.clone(), &path, None).await;
                assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} {method}");
            }
        }
        assert_eq!(image_deleted(app.db(), img1).await, Some(false));

        // Deleted first, so a restore that got through would show.
        let (status, _) = app.expect(Caller::Coach, Method::DELETE, &path, None).await;
        assert_eq!(status, StatusCode::OK);
        for caller in NON_PARTICIPANTS {
            let (status, _) = app
                .expect(caller, Method::POST, &format!("{path}/restore"), None)
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{caller:?} restore");
        }
        assert_eq!(image_deleted(app.db(), img1).await, Some(true));

        // The matrix's note: an unknown image is not found.
        let (status, _) = app
            .expect(
                Caller::Coach,
                Method::GET,
                &format!("/coaching_session_images/{}", Id::new_v4()),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    })
    .await;
}
