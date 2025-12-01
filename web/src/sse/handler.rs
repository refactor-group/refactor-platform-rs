use crate::extractors::authenticated_user::AuthenticatedUser;
use async_stream::stream;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use log::*;
use service::AppState;
use std::convert::Infallible;
use tokio::sync::mpsc;

/// SSE handler that establishes a long-lived connection for real-time updates.
/// One connection per authenticated user, stays open across page navigation.
pub(crate) async fn sse_handler(
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    debug!("Establishing SSE connection for user {}", user.id);

    let (tx, mut rx) = mpsc::unbounded_channel();

    // Register returns the connection_id (convert domain::Id to String)
    let connection_id = app_state
        .sse_manager
        .register_connection(user.id.to_string(), tx);

    let manager = app_state.sse_manager.clone();
    let user_id = user.id;

    // Create the stream - events arrive from the channel
    // The channel sends Result<Event, Infallible>, so we just pass them through
    let stream = stream! {
        while let Some(event) = rx.recv().await {
            yield event;
        }

        // Connection closed, clean up
        debug!("SSE connection closed for user {}, cleaning up", user_id);
        manager.unregister_connection(&connection_id);
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}
