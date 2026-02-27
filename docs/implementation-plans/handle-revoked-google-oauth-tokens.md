# Plan: Handle Revoked Google OAuth Refresh Tokens

## Context

When a Google refresh token is permanently revoked (`invalid_grant`), the current code returns
HTTP 500/502 and leaves the stale `oauth_connections` row intact. The UI shows the user as
connected even though no API calls can succeed. The fix threads a distinct `TokenRevoked` error
through four layers so that revocation: (1) automatically cleans up the DB row, (2) returns
HTTP 401 to the caller, and (3) prompts the user to reconnect on the frontend.

---

## Changes

### 1. `meeting-auth/src/error.rs`

Add `TokenRevoked` to `OAuthErrorKind` — permanent invalidation, distinct from a transient
`TokenRefreshFailed`:

```rust
pub enum OAuthErrorKind {
    AuthorizationFailed,
    TokenExchangeFailed,
    TokenRefreshFailed,
    TokenRevoked,           // permanent: invalid_grant from provider
    RevocationFailed,
    ...
}
```

### 2. `meeting-auth/src/oauth/providers/google.rs`

In the `refresh_token()` error branch, parse the response body for `invalid_grant` and return
`TokenRevoked` instead of the generic `TokenRefreshFailed`:

```rust
} else {
    let error_text = response.text().await.unwrap_or_default();
    warn!("Google token refresh error: {}", error_text);
    if error_text.contains("invalid_grant") {
        Err(oauth_error(OAuthErrorKind::TokenRevoked, &error_text))
    } else {
        Err(oauth_error(OAuthErrorKind::TokenRefreshFailed, &error_text))
    }
}
```

### 3. `domain/src/error.rs`

Add `OauthTokenRevoked` to `ExternalErrorKind` — token revocation is an external failure
(the provider revoked credentials), not an internal entity auth issue:

```rust
pub enum ExternalErrorKind {
    Network,
    OauthTokenRevoked,      // provider permanently revoked the refresh token
    Other(String),
}
```

Add `OAuthErrorKind` to the import and update `From<MeetingAuthError>` to map `TokenRevoked`
to the new variant:

```rust
use meeting_auth::error::{Error as MeetingAuthError, ErrorKind as MeetingAuthErrorKind, OAuthErrorKind};

// In From<MeetingAuthError>:
MeetingAuthErrorKind::OAuth(OAuthErrorKind::TokenRevoked) => {
    DomainErrorKind::External(ExternalErrorKind::OauthTokenRevoked)
}
MeetingAuthErrorKind::OAuth(_) => {
    DomainErrorKind::External(ExternalErrorKind::Other("OAuth error".to_string()))
}
```

### 3a. `web/src/error.rs`

Add `OauthTokenRevoked` to `handle_external_error` → HTTP 401:

```rust
ExternalErrorKind::OauthTokenRevoked => {
    warn!("ExternalErrorKind::OauthTokenRevoked: Responding with 401 Unauthorized. Error: {self:?}");
    (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
}
```

### 4. `domain/src/oauth_connection.rs` — `get_valid_access_token`

Replace the `?` propagation on `manager.get_valid_token` with an explicit match. On revocation,
delete the `oauth_connections` row before returning the error so that
`GET /oauth/connections/google` immediately returns 404:

```rust
let result = manager
    .get_valid_token(&oauth_provider, &user_id.to_string())
    .await
    .inspect_err(|e| warn!("Failed to get valid token for user {}: {:?}", user_id, e));

match result {
    Ok(token) => Ok(token.expose_secret().to_string()),
    Err(ref e)
        if matches!(
            e.error_kind,
            meeting_auth::error::ErrorKind::OAuth(OAuthErrorKind::TokenRevoked)
        ) =>
    {
        warn!("Refresh token revoked for user {}, removing connection", user_id);
        // Use the provider param (not hardcoded Google) for future provider support
        let _ = entity_api::oauth_connection::delete_by_user_and_provider(
            db,
            user_id,
            provider,
        )
        .await;
        Err(result.unwrap_err().into())
    }
    Err(e) => Err(e.into()),
}
```

Also rename `_provider` → `provider` in the function signature since it is now used.
The `From<MeetingAuthError>` impl (step 3) converts this into `OauthTokenRevoked` → HTTP 401.
The `let _ =` on the delete intentionally ignores errors (row may already be absent).

### 5. Frontend — `google-integration-section.tsx`

`handleCreateMeet` currently catches all errors with a generic toast. Add a 401 check to
navigate to the settings/integrations page (which contains the Google OAuth connect button)
and show a toast there. Since Sonner's toast state is global and persists across Next.js
client-side navigations, the toast renders on the destination page:

```typescript
} catch (err) {
    if (err instanceof EntityApiError && err.status === 401) {
        await refresh(); // flips connection state to null
        router.push("/settings/integrations");
        toast.error("Oauth connection revoked. You must reconnect");
    } else {
        toast.error("Failed to create Google Meet link.");
    }
}
```

Add `const router = useRouter();` to the component (import from `next/navigation`).
Also import `EntityApiError` from `@/lib/api/entity-api`.

---

## Error propagation path (after this change)

```
Google returns 400 {"error": "invalid_grant"}
  → meeting-auth:  OAuthErrorKind::TokenRevoked
  → domain:        intercepts, deletes oauth_connections row (using provider param), returns MeetingAuthError
  → From impl:     OAuth(TokenRevoked) → ExternalErrorKind::OauthTokenRevoked
  → web:           ExternalErrorKind::OauthTokenRevoked → HTTP 401
  → frontend:      catches 401, calls refresh(), navigates to /settings/integrations, shows "Oauth connection revoked. You must reconnect" toast
  → GET /oauth/connections/google → 404 (row deleted)
  → UI: shows "Connect Google Account" button
```

---

## Files changed

- `meeting-auth/src/error.rs`
- `meeting-auth/src/oauth/providers/google.rs`
- `domain/src/error.rs`
- `domain/src/oauth_connection.rs`
- `web/src/error.rs`
- `refactor-platform-fe/src/components/ui/settings/google-integration-section.tsx`

---

## Verification

1. `cargo check` — clean
2. `cargo clippy` — no warnings
3. `cargo test --features mock` — all tests pass
4. Manual: simulate revocation by revoking app access from Google account settings
   (`myaccount.google.com/permissions`), then attempt meeting creation — should receive
   401, UI should flip to disconnected state with reconnect prompt
