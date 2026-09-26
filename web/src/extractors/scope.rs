//! Search visibility-scope extractor.
//!
//! `/search` has no resource id in its path, so the per-resource access
//! extractors don't apply; authorization is embedded in the search queries
//! instead. This extractor plays the access extractors' role at the caller
//! end: the handler receives its authorization input — the compiled `Scope` —
//! the same way resource handlers receive their access proofs. Compilation is
//! pure (preloaded roles, zero queries).

use axum::{async_trait, extract::FromRequestParts, http::request::Parts};
use domain::coaching_relationships;
use domain::search::scope_for;

use crate::extractors::{authenticated_user::AuthenticatedUser, RejectionType};

pub(crate) struct Scope(pub coaching_relationships::Scope);

#[async_trait]
impl<S> FromRequestParts<S> for Scope
where
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, state).await?;
        Ok(Scope(scope_for(&user)))
    }
}
