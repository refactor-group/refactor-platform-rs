pub(crate) struct ResourceAccess(pub Id);

#[async_trait]
impl<S> FromRequestParts<S> for ResourceAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
      let state = AppState::
        // 1. pull AppState out of S
        // 2. parse the path id
        // 3. compose AuthenticatedUser
        // 4. confirm the resource exists (DB query)
        // 5. check the role
        // 6. return Ok(Self(id)) or Err(...)
    }
}
