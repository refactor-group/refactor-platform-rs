//! `DELETE /coaching_session_series/:id`, driven through the real router with a logged-in
//! session and a mock database: the coach gets past the coach-only gate. Deleting a series
//! with upcoming sessions takes Postgres advisory locks and is checked by hand.

use std::sync::Arc;

use axum::{body::Body, extract::Request, http::StatusCode, routing::post, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use sea_orm::MockExecResult;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use domain::user::Backend;
use service::config::Config;

use crate::controller::resend_invite_tests::{start_in, Call, Fixture, Series, Session};
use crate::controller::user_session_controller;
use crate::router::coaching_session_series_routes;
use crate::AppState;

/// Logs the caller in over the real router, sends the `DELETE`, and returns the status and
/// every DELETE statement run.
async fn delete(call: Call) -> (StatusCode, Vec<String>) {
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
        .merge(coaching_session_series_routes(app_state))
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
    drop(response);
    let log = Arc::try_unwrap(db).expect("the app has been dropped");
    let deletes = log
        .into_transaction_log()
        .iter()
        .flat_map(|txn| txn.statements())
        .map(|stmt| stmt.sql.clone())
        .filter(|sql| sql.starts_with("DELETE"))
        .collect();
    (status, deletes)
}

/// SER-12
#[tokio::test]
async fn the_coach_gets_past_the_gate_to_delete_a_series() {
    let f = Fixture::new();
    let series = f.series(start_in(-30));
    let path = format!("/coaching_session_series/{}", series.id);
    // The handler's loads for a series with no upcoming sessions, so no session lock is
    // taken: the upcoming sessions, read again in the transaction, a SEQUENCE bump that
    // returns no row (no cancellation is mailed), then the series delete.
    let call = f.gate(&f.coach, &series, path).then(|db| {
        db.append_query_results([Vec::<Session>::new()])
            .append_query_results([Vec::<Session>::new()])
            .append_query_results([Vec::<Series>::new()])
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
    });

    let (status, deletes) = delete(call).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    let [delete] = deletes.as_slice() else {
        panic!("expected the series delete alone, got {deletes:?}");
    };
    assert!(
        delete.starts_with(r#"DELETE FROM "refactor_platform"."coaching_session_series""#),
        "{delete}"
    );
}
