use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use log::*;

use crate::AppState;

/// How long the readiness check waits on the database, pool queue included.
const DATABASE_TIMEOUT: Duration = Duration::from_secs(2);

/// GET liveness probe for the API server
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "API router is up and responding to requests", body = String),
        (status = 500, description = "Internal Server Error")
    )
)]
pub async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "healthy")
}

/// GET readiness probe for the API server
///
/// Answers 200 only when the server can reach the database within two seconds,
/// so a database outage or an exhausted connection pool shows up as 503.
#[utoipa::path(
    get,
    path = "/health/ready",
    responses(
        (status = 200, description = "API server is up and can reach the database", body = String),
        (status = 429, description = "Too many requests from this IP"),
        (status = 503, description = "API server cannot reach the database", body = String)
    )
)]
pub async fn ready(State(app_state): State<AppState>) -> impl IntoResponse {
    match tokio::time::timeout(DATABASE_TIMEOUT, app_state.db_conn_ref().ping()).await {
        Ok(Ok(())) => (StatusCode::OK, "ready"),
        Ok(Err(err)) => {
            warn!("Readiness check could not reach the database: {err}");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready")
        }
        Err(_) => {
            warn!("Readiness check timed out waiting for the database");
            (StatusCode::SERVICE_UNAVAILABLE, "not ready")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use sea_orm::{Database, DatabaseConnection};
    use service::config::Config;
    use tower::ServiceExt;

    use super::*;

    async fn ready_status(db: DatabaseConnection) -> StatusCode {
        let app = Router::new()
            .route("/health/ready", get(ready))
            .with_state(AppState::new(
                service::AppState::new(Config::default(), &Arc::new(db)),
                Arc::new(sse::Manager::default()),
                domain::events::EventPublisher::new(),
                None,
                None,
                None,
            ));
        let request = Request::get("/health/ready")
            .body(Body::empty())
            .expect("the request builds");

        app.oneshot(request)
            .await
            .expect("the router answers")
            .status()
    }

    #[tokio::test]
    async fn ready_when_the_database_answers() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory SQLite connects");

        assert_eq!(ready_status(db).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn not_ready_when_the_database_is_unreachable() {
        assert_eq!(
            ready_status(DatabaseConnection::default()).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
