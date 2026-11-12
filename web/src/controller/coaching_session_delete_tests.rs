//! `DELETE /coaching_sessions/:id`, driven through the real router with a logged-in
//! session and a mock database. Which sessions are refused is covered in `domain`.

use std::sync::Arc;

use axum::{body::Body, extract::Request, http::StatusCode, routing::post, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DbErr};
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use domain::user::Backend;
use domain::{coaching_session_images, coaching_sessions};
use service::config::Config;

use super::resend_invite_tests::{start_in, Call, Fixture};
use crate::controller::user_session_controller;
use crate::router::coaching_sessions_routes;
use crate::AppState;

/// A real foreign key violation from in-memory SQLite, for the mock to return.
async fn foreign_key_violation() -> DbErr {
    let options = ConnectOptions::new("sqlite::memory:")
        .max_connections(1)
        .to_owned();
    let db = Database::connect(options)
        .await
        .expect("in-memory SQLite opens");
    for statement in [
        "PRAGMA foreign_keys = ON",
        "CREATE TABLE parent (id INTEGER PRIMARY KEY)",
        "CREATE TABLE child (parent_id INTEGER REFERENCES parent (id))",
        "INSERT INTO parent VALUES (1)",
        "INSERT INTO child VALUES (1)",
    ] {
        db.execute_unprepared(statement)
            .await
            .expect("the tables are seeded");
    }
    db.execute_unprepared("DELETE FROM parent")
        .await
        .expect_err("the child's foreign key refuses the delete")
}

/// Sends the call's `DELETE` and returns the status and the JSON body, if any.
async fn delete(call: Call) -> (StatusCode, serde_json::Value) {
    let db = Arc::new(call.db.into_connection());
    let app_state = AppState::new(
        service::AppState::new(Config::default(), &db),
        Arc::new(sse::Manager::default()),
        domain::events::EventPublisher::default(),
        None,
        None,
        None,
    );
    let session_layer = SessionManagerLayer::new(MemoryStore::default())
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::days(1)))
        .with_always_save(true);
    let app = Router::new()
        .route("/login", post(user_session_controller::login))
        .with_state(app_state.clone())
        .merge(coaching_sessions_routes(app_state))
        .layer(AuthManagerLayerBuilder::new(Backend::new(&db), session_layer).build());

    let mut request = Request::delete(call.path).header("x-version", "1.0.0");
    if let Some(caller) = call.caller {
        let form = format!("email={caller}&password=password123");
        let login = Request::post("/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(form))
            .unwrap();
        let login = app.clone().oneshot(login).await.unwrap();
        request = request.header("cookie", login.headers()["set-cookie"].clone());
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();

    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
async fn sd_06_a_refused_delete_answers_409_with_its_reason() {
    let f = Fixture::new();
    let session = f.session(start_in(7));
    let bumped = coaching_sessions::Model {
        ical_sequence: session.ical_sequence + 1,
        ..session.clone()
    };
    let path = format!("/coaching_sessions/{}", session.id);
    let violation = foreign_key_violation().await;
    // The coach gate, then the domain's read, the image keys, the bump and the delete.
    let call = f.login(&f.coach, path).then(|db| {
        db.append_query_results([vec![(session.clone(), f.relationship.clone())]])
            .append_query_results([vec![session.clone()]])
            .append_query_results([Vec::<coaching_session_images::Model>::new()])
            .append_query_results([vec![bumped]])
            .append_exec_errors([violation])
    });

    let (status, body) = delete(call).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["details"]["reason"], "sessions_have_content", "{body}");
    assert_eq!(
        body["details"]["coaching_session_id"],
        serde_json::json!(session.id),
        "{body}"
    );
}
