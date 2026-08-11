# Test Plan: Manually Testing OAuth Disconnect Revocation

Verify that disconnecting a meeting integration revokes the grant with the
provider, not just the row in our database.

The single most important property: **the provider must forget us.** Before this
change, Disconnect deleted the `oauth_connections` row and returned, leaving the
access and refresh tokens valid and the grant listed on the coach's provider
account page. Our app showing "not connected" is **not** evidence of revocation.
The provider's own permissions page is the only evidence that counts.

> [!IMPORTANT]
> No automated test reaches the wire. Providers are built from config rather than
> injected, so the suite covers token selection and the delete but never an actual
> `revoke_token` call. These scenarios are the only proof revocation works.

## 1. Prerequisites

- Backend on branch `fix/oauth-disconnect-revokes-token`, migrations applied.
- `ENCRYPTION_KEY` set, plus `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` /
  `GOOGLE_REDIRECT_URI` for Scenario A and the `ZOOM_*` equivalents for Scenario B.
- A real Google account, and a real Zoom account for Scenario B.
- Backend logs visible, since a failed revoke is only ever a `warn!`.

## 2. Scenario A: Google disconnect revokes the grant

1. Connect Google in Settings > Integrations.
2. Open https://myaccount.google.com/permissions and confirm the app is listed.
3. Hit Disconnect in the app.
4. Reload the Google permissions page.

**Pass:** the app is gone from the permissions page, and the connection no longer
appears in Settings > Integrations.

**Result: PASS, verified 2026-08-11.** The grant was removed from the Google
account permissions page after disconnecting.

## 3. Scenario B: Zoom disconnect revokes the grant

Same shape as Scenario A, against Zoom's app management page
(https://marketplace.zoom.us > Manage > Added Apps).

**Result: BLOCKED. Zoom OAuth is not configured in any environment.**

`GET /oauth/zoom/authorize` returns 500 because `create_zoom_provider` needs
`ZOOM_CLIENT_ID`, `ZOOM_CLIENT_SECRET` and `ZOOM_REDIRECT_URI`, and none of the
three appear anywhere in the repository: not in `.env`, `docker-compose.yaml`,
`docker-compose.pr-preview.yaml`, `deploy_to_do.yml`,
`ci-deploy-pr-preview.yml` or `docs/setup.md`. Their Google counterparts are in
all six. There is no Zoom connection to disconnect, so revocation cannot be
exercised.

Enabling Zoom means creating a Zoom Marketplace app and wiring those three
variables through every layer. Until then Zoom revocation ships **unverified**.

When Zoom is enabled, run this scenario before trusting it. Zoom's revoke
endpoint authenticates the client with HTTP Basic, and the request previously
sent no client credentials at all, so it would have failed on every call. That
code was unreachable until `revoke_token` gained a caller. The fix follows
Zoom's documented requirement but has never been observed working, and because
failures are `warn!` only, a still-broken revoke looks identical to success in
the UI.

**Pass:** the app is gone from Added Apps, and the log shows
`Revoked zoom grant for user <id>` rather than `Failed to revoke`.

## 4. Scenario C: A failed revoke still disconnects the user

Revocation is best effort. A provider we cannot reach must not strand the user in
a connected-but-broken state.

1. Connect a provider.
2. Make revocation fail: disconnect the host from the network, or point the
   provider's config at an unroutable revoke URL and restart the backend.
3. Hit Disconnect.

**Pass:** the app reports success, the connection is gone from
Settings > Integrations, the row is gone from `oauth_connections`, and the log
carries a `Failed to revoke` or `Cannot revoke` warning.

**Result: NOT YET RUN.**

## 5. Known limitation

Anyone who disconnected before this shipped still has a live grant at the provider
and no row left for us to revoke from. They have to revoke manually from the
provider's account permissions page.
