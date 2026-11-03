//! `POST /coaching_session_series/:id/reschedule`, driven through the real router with a
//! logged-in session and a mock database. Which rows move is covered in `domain`.

use std::sync::Arc;

use axum::{body::Body, extract::Request, http::StatusCode, routing::post, Router};
use axum_login::{
    tower_sessions::{MemoryStore, SessionManagerLayer},
    AuthManagerLayerBuilder,
};
use sea_orm::{MockExecResult, Value};
use serde_json::json;
use time::Duration;
use tower::ServiceExt;
use tower_sessions::Expiry;

use domain::user::Backend;
use domain::Id;
use service::config::Config;

use super::resend_invite_tests::{start_in, Call, Fixture, Series, Session};
use crate::controller::user_session_controller;
use crate::router::coaching_session_series_routes;
use crate::AppState;

fn exec_ok() -> MockExecResult {
    MockExecResult {
        last_insert_id: 0,
        rows_affected: 1,
    }
}

fn path(series: &Series) -> String {
    format!("/coaching_session_series/{}/reschedule", series.id)
}

/// A content or lock row: just the session id.
fn session_id_row(id: Id) -> std::collections::BTreeMap<&'static str, Value> {
    std::collections::BTreeMap::from([("coaching_session_id", Value::from(id))])
}

impl Fixture {
    /// Past the gate, the domain's reads before any lock: the coach's zone, the series and
    /// its sessions.
    fn planned(&self, series: &Series, rows: &[Session]) -> Call {
        self.gate(&self.coach, series, path(series)).then(|db| {
            db.append_query_results([vec![self.coach.clone()]])
                .append_query_results([vec![series.clone()]])
                .append_query_results([rows.to_vec()])
        })
    }

    /// Planned, then the transaction's session locks and its locked re-reads.
    fn locked(&self, series: &Series, rows: &[Session], locks: usize) -> Call {
        self.planned(series, rows).then(|db| {
            db.append_exec_results(vec![exec_ok(); 1 + locks])
                .append_query_results([vec![series.clone()]])
                .append_query_results([rows.to_vec()])
        })
    }
}

/// Posts `body` to the call's path and returns the status and the JSON body, if any.
async fn post_json(call: Call, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
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

    let mut request = Request::post(call.path)
        .header("x-version", "1.0.0")
        .header("content-type", "application/json");
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
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
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

fn move_to(session: &Session, start_at: chrono::NaiveDateTime) -> serde_json::Value {
    json!({ "coaching_session_id": session.id, "start_at": start_at })
}

/// A request that leaves every session where it is answers with the series and the
/// sessions it covers, in the series read's envelope.
#[tokio::test]
async fn a_reschedule_that_changes_nothing_returns_the_series_and_its_sessions() {
    let f = Fixture::new();
    let series = f.series(start_in(7));
    let rows = f.members(&series);

    let (status, body) =
        post_json(f.locked(&series, &rows, 2), move_to(&rows[1], rows[1].date)).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["series"]["id"], json!(series.id));
    let ids: Vec<_> = body["data"]["sessions"]
        .as_array()
        .expect("sessions")
        .iter()
        .map(|session| session["id"].clone())
        .collect();
    assert_eq!(ids, [json!(rows[1].id), json!(rows[2].id)]);
}

/// Leaving out a session that has content is refused with the reason and the sessions
/// in the way.
#[tokio::test]
async fn leaving_out_a_session_with_content_is_a_409_naming_it() {
    let f = Fixture::new();
    let series = f.series(start_in(7));
    let rows = f.members(&series);
    let call = f.locked(&series, &rows, 3).then(|db| {
        db.append_query_results([vec![session_id_row(rows[1].id), session_id_row(rows[2].id)]])
            .append_query_results([vec![session_id_row(rows[2].id)]])
    });

    let body = json!({
        "coaching_session_id": rows[0].id,
        "start_at": rows[0].date,
        "recurrence": { "frequency": "weekly", "count": 1 },
    });
    let (status, body) = post_json(call, body).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "conflict");
    assert_eq!(body["details"]["reason"], "sessions_have_content");
    assert_eq!(body["details"]["coaching_session_ids"], json!([rows[2].id]));
}

/// A session from outside the series is a validation error, refused before any lock.
#[tokio::test]
async fn a_session_outside_the_series_is_a_422() {
    let f = Fixture::new();
    let series = f.series(start_in(7));
    let rows = f.members(&series);
    let stranger = f.session(start_in(8));

    let (status, body) =
        post_json(f.planned(&series, &rows), move_to(&stranger, start_in(8))).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"], "validation_error");
}

/// A body missing `start_at` never reaches the domain.
#[tokio::test]
async fn a_body_without_a_start_is_refused_by_the_route() {
    let f = Fixture::new();
    let series = f.series(start_in(7));
    let rows = f.members(&series);

    let call = f.gate(&f.coach, &series, path(&series));
    let (status, _) = post_json(call, json!({ "coaching_session_id": rows[0].id })).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

/// Only the series' coach may reschedule it; no cookie is a 401 and an unknown series a
/// 404.
#[tokio::test]
async fn the_route_is_gated_like_the_other_series_writes() {
    let f = Fixture::new();
    let series = f.series(start_in(7));
    let rows = f.members(&series);
    let unknown = format!("/coaching_session_series/{}/reschedule", Id::new_v4());
    let body = move_to(&rows[0], rows[0].date);

    let cases = [
        (
            f.gate(&f.coachee, &series, path(&series)),
            StatusCode::FORBIDDEN,
        ),
        (Fixture::anonymous(path(&series)), StatusCode::UNAUTHORIZED),
        (
            f.login(&f.coach, unknown)
                .then(|db| db.append_query_results([Vec::<Series>::new()])),
            StatusCode::NOT_FOUND,
        ),
    ];

    for (case, (call, expected)) in cases.into_iter().enumerate() {
        let (status, _) = post_json(call, body.clone()).await;
        assert_eq!(status, expected, "case {case}");
    }
}
