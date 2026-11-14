use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};
use domain::{coaching_session, coaching_session_topic, coaching_session_topics};

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser, coaching_session_access::CoachingSessionAccess,
        lookup_rejection, not_found, parse_path_id_from_parts, RejectionType,
    },
    AppState,
};

/// Verifies the authenticated user is a participant of the path session AND that the
/// `:topic_id` topic belongs to that session.
///
/// Composes `CoachingSessionAccess` (participant + session check), then loads the topic
/// and confirms it belongs to the path session. A non-participant gets 403; a topic missing
/// from the path session is 404, so a topic in another session is never revealed. On
/// success, yields the topic model.
pub(crate) struct CoachingSessionTopicAccess(pub coaching_session_topics::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionTopicAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        // Composes the participant + session check (reuses the tested extractor).
        let CoachingSessionAccess(session) =
            CoachingSessionAccess::from_request_parts(parts, state).await?;

        let topic_id = parse_path_id_from_parts(parts, "topic_id").await?;

        // Load + verify the topic belongs to THIS session (else 404 to hide existence).
        let topic = coaching_session_topic::find_by_id(app_state.db_conn_ref(), topic_id)
            .await
            .map_err(|e| lookup_rejection(e.into()))?;

        if topic.coaching_session_id != session.id {
            return Err(not_found());
        }

        Ok(CoachingSessionTopicAccess(topic))
    }
}

/// Authorizes a topic delete. Composes `CoachingSessionTopicAccess` (participant + topic belongs
/// to the path session), then allows the caller only if they are the topic's author OR the coach
/// of the session's relationship. So a coach may delete any topic in the session (including a
/// coachee's), while a coachee may delete only their own. Any other participant gets 404.
pub(crate) struct CoachingSessionTopicDeleteAccess(pub coaching_session_topics::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionTopicDeleteAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let CoachingSessionTopicAccess(topic) =
            CoachingSessionTopicAccess::from_request_parts(parts, state).await?;

        let AuthenticatedUser(user) =
            AuthenticatedUser::from_request_parts(parts, &app_state).await?;

        // The author may delete their own topic.
        if topic.user_id == user.id {
            return Ok(CoachingSessionTopicDeleteAccess(topic));
        }

        // Otherwise only the coach of the session's relationship may delete it.
        let (_session, relationship) = coaching_session::find_by_id_with_coaching_relationship(
            app_state.db_conn_ref(),
            topic.coaching_session_id,
        )
        .await
        .map_err(lookup_rejection)?;

        if relationship.coach_id == user.id {
            return Ok(CoachingSessionTopicDeleteAccess(topic));
        }

        Err(not_found())
    }
}

/// Authorizes an undo. Composes CoachingSessionAccess (participant + path session), loads the
/// topic INCLUDING soft-deleted, and confirms it belongs to the path session. Undoing a delete
/// (the topic is soft-deleted) additionally requires the caller to be the author. A non-participant
/// gets 403; a topic missing from the path session, or another author's deleted topic, is 404.
///
/// Note: the session-match check is against the topic's CURRENT session. After a defer-move the
/// topic lives in the destination session, so undo must be called under the destination session's
/// URL; calling it under the origin's (where the topic no longer appears) yields 404.
pub(crate) struct CoachingSessionTopicUndoAccess(pub coaching_session_topics::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionTopicUndoAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let CoachingSessionAccess(session) =
            CoachingSessionAccess::from_request_parts(parts, state).await?;

        let topic_id = parse_path_id_from_parts(parts, "topic_id").await?;

        let topic =
            coaching_session_topic::find_including_deleted_by_id(app_state.db_conn_ref(), topic_id)
                .await
                .map_err(|e| lookup_rejection(e.into()))?;

        if topic.coaching_session_id != session.id {
            return Err(not_found());
        }

        // Undoing a delete is author-only; undoing a defer is open to either participant.
        if topic.deleted_at.is_some() {
            let AuthenticatedUser(user) =
                AuthenticatedUser::from_request_parts(parts, &app_state).await?;
            if topic.user_id != user.id {
                return Err(not_found());
            }
        }

        Ok(CoachingSessionTopicUndoAccess(topic))
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use axum::{http::StatusCode, routing::get, Router};
    use sea_orm::DbErr;

    use super::*;
    use crate::extractors::relationship_access_test_support::{self as support, Caller};

    async fn protected_route(_access: CoachingSessionTopicAccess) -> &'static str {
        "extracted_success"
    }

    #[tokio::test]
    async fn a_failed_topic_lookup_is_a_server_error() {
        let w = support::fixture();
        let db = Arc::new(
            w.signed_in(Caller::Coach)
                .append_query_results([vec![(w.session.clone(), w.relationship.clone())]])
                .append_query_errors([DbErr::Custom("connection lost".to_string())])
                .into_connection(),
        );
        let routes = Router::new().route(
            "/coaching_sessions/:coaching_session_id/topics/:topic_id",
            get(protected_route),
        );

        let response = support::send(
            routes,
            &db,
            &w.coach.email,
            "GET",
            &format!(
                "/coaching_sessions/{}/topics/{}",
                w.session.id,
                domain::Id::new_v4()
            ),
            None,
        )
        .await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
