//! Search access extractor.
//!
//! `/search` has no resource id in its path, so the per-resource access
//! extractors don't apply; authorization is embedded in the search queries
//! instead. This extractor plays the access extractors' role at the caller
//! end: the handler receives its authorization input — the caller's compiled
//! `Visibility` — the same way resource handlers receive their access proofs.
//! Compilation is pure (preloaded roles, zero queries).

use axum::{async_trait, extract::FromRequestParts, http::request::Parts};
use domain::coaching_relationships::Visibility;
use domain::search::visibility_for;

use crate::extractors::{authenticated_user::AuthenticatedUser, RejectionType};

pub(crate) struct SearchAccess(pub Visibility);

#[async_trait]
impl<S> FromRequestParts<S> for SearchAccess
where
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, state).await?;
        Ok(SearchAccess(visibility_for(&user)))
    }
}
