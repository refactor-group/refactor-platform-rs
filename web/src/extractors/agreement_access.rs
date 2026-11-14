use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};
use domain::{agreement as AgreementApi, agreements};

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser, coaching_session_access::authorize_session,
        lookup_rejection, parse_path_id_from_parts, RejectionType,
    },
    AppState,
};

/// Verifies the authenticated user is a participant of the relationship that owns the `:id`
/// agreement, by way of the agreement's coaching session.
///
/// An unknown agreement is a 404; an unknown session is a 404 and a non-participant a 403, as
/// for [`authorize_session`]. On success, yields the agreement.
pub(crate) struct AgreementAccess {
    pub agreement: agreements::Model,
}

#[async_trait]
impl<S> FromRequestParts<S> for AgreementAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, state).await?;

        let agreement_id = parse_path_id_from_parts(parts, "id").await?;

        let agreement = AgreementApi::find_by_id(app_state.db_conn_ref(), agreement_id)
            .await
            .map_err(|e| lookup_rejection(e.into()))?;

        authorize_session(&app_state, &user, agreement.coaching_session_id).await?;

        Ok(AgreementAccess { agreement })
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

    async fn protected_route(_access: AgreementAccess) -> &'static str {
        "extracted_success"
    }

    #[tokio::test]
    async fn a_failed_agreement_lookup_is_a_server_error() {
        let w = support::fixture();
        let db = Arc::new(
            w.signed_in(Caller::Coach)
                .append_query_errors([DbErr::Custom("connection lost".to_string())])
                .into_connection(),
        );
        let routes = Router::new().route("/agreements/:id", get(protected_route));

        let response = support::send(
            routes,
            &db,
            &w.coach.email,
            "GET",
            &format!("/agreements/{}", domain::Id::new_v4()),
            None,
        )
        .await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
