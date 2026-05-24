# Platform Setup Guide

This guide covers setting up the Refactor platform for local development and production deployment.

---

## Development Setup

### Prerequisites

- Rust toolchain (`rustup` + stable)
- PostgreSQL 14+ (see [README.md](../README.md) for DB setup)
- `cargo`, `sea-orm-cli`
- [ngrok](https://ngrok.com/) or similar tunnel (for Recall.ai webhooks)

### 1. Core Application

Follow the database setup and backend startup instructions in [README.md](../README.md) first. The steps below layer on the credentials needed for meeting transcription.

### 2. Encryption Key

All OAuth tokens are encrypted at rest. This key must be set before any OAuth flow works.

```bash
# Generate a 64-hex-character key (32 random bytes, hex-encoded):
openssl rand -hex 32
```

> `openssl rand -hex 32` outputs **32 bytes encoded as 64 hex characters** — this is correct for `ENCRYPTION_KEY`.

```env
ENCRYPTION_KEY=<64-hex-char output from above>
```

### 3. Google OAuth

#### Create a Google Cloud Project

1. Go to [console.cloud.google.com](https://console.cloud.google.com) and create a new project (or use an existing one).
2. Enable these two APIs:
   - Google Meet API (`meet.googleapis.com`)
   - Google People API (`people.googleapis.com`)
3. Navigate to **APIs & Services → OAuth consent screen**:
   - User Type: **External**
   - Fill in App name, support email, developer contact
   - Add scopes:
     - `openid`
     - `email`
     - `profile`
     - `https://www.googleapis.com/auth/meetings.space.created`
   - Add your Google account as a **Test user** (required while the app is in "Testing" status)
4. Navigate to **APIs & Services → Credentials → Create Credentials → OAuth 2.0 Client ID**:
   - Application type: **Web application**
   - Authorized redirect URI: `http://localhost:4000/api/auth/google/callback`
   - Download or copy the Client ID and Client Secret

#### Environment Variables

```env
GOOGLE_CLIENT_ID=<client-id>.apps.googleusercontent.com
GOOGLE_CLIENT_SECRET=<client-secret>
GOOGLE_REDIRECT_URI=http://localhost:4000/api/auth/google/callback

# Note: this var has NO `GOOGLE_` prefix even though it's used by the Google OAuth flow.
# A typo like `GOOGLE_OAUTH_SUCCESS_REDIRECT_URI=...` is silently ignored (the code
# falls back to the default `http://localhost:3000/settings`).
OAUTH_SUCCESS_REDIRECT_URI=http://localhost:3000/settings

# These have working defaults — only set if you need to override:
# GOOGLE_OAUTH_AUTH_URL=https://accounts.google.com/o/oauth2/v2/auth
# GOOGLE_OAUTH_TOKEN_URL=https://oauth2.googleapis.com/token
# GOOGLE_USERINFO_URL=https://www.googleapis.com/oauth2/v2/userinfo
# GOOGLE_MEET_API_URL=https://meet.googleapis.com/v2
```

### 4. Recall.ai

#### Get an API Key

1. Sign up at [recall.ai](https://recall.ai) and create a workspace.
2. In the dashboard, navigate to **API Keys** and generate a new key.
3. Note your region — use `us-east-1` for US workspaces, `eu-west-2` for EU.

#### Configure a Webhook

Recall.ai sends bot lifecycle events to `POST /webhooks/recall_ai`. For local development the backend must be publicly reachable — use ngrok:

```bash
# Start the backend first, then in another terminal.

# Free plan (random subdomain, changes on every restart):
ngrok http 4000
# Copy the https URL from the output, e.g. https://abc123.ngrok-free.dev

# Paid plan (reserved domain that survives restarts):
ngrok http 4000 --url=<your-stable-domain>.ngrok-free.dev
```

> Note: ngrok deprecated the `--domain` flag. Use `--url` for reserved domains. Plain `ngrok http 4000` still works for free-plan ephemeral URLs.

In the Recall.ai dashboard:

1. Go to **Webhooks → Add Endpoint**
2. URL: `https://<your-ngrok-url>/webhooks/recall_ai`
3. Select events: `bot.status_change`, `recording.done`, `transcript.done`
4. After saving, copy the **Signing Secret** (starts with `whsec_`)

> **Important: the `/webhooks/recall_ai` path is required.**
>
> Setting the URL to just the ngrok host (e.g. `https://abc123.ngrok-free.dev`) causes every delivery to fail with **405 Method Not Allowed**, because the Axum router falls back to a static-file handler that only accepts GET/HEAD for unmatched paths.
>
> - Wrong: `https://abc123.ngrok-free.dev`
> - Right: `https://abc123.ngrok-free.dev/webhooks/recall_ai`

> **Note:** The ngrok URL changes every restart on the free plan. Update the Recall.ai webhook endpoint each session, or use a paid ngrok plan with a stable URL.

#### Verify the endpoint

Before triggering a real Recall.ai bot, send a raw POST to confirm the path resolves to the handler:

```bash
curl -i -X POST https://<your-ngrok-url>/webhooks/recall_ai
```

Interpret the response:

- **401 Unauthorized** (body: `Webhook secret not configured` or `Invalid signature`) means the request reached the Axum handler. This is the **expected** response for a raw curl, since it has no valid Svix signature. The URL is correct.
- **405 Method Not Allowed** means the URL path is wrong and the request was absorbed by the static-file fallback. Re-check that the endpoint in the Recall.ai dashboard ends in `/webhooks/recall_ai`.
- **404 Not Found** or no response means ngrok isn't tunneling to your backend, or the backend isn't running on port 4000.

#### Environment Variables

```env
RECALL_AI_API_KEY=<your-recall-ai-api-key>
RECALL_AI_REGION=us-east-1          # or eu-west-2
RECALL_AI_WEBHOOK_SECRET=whsec_<base64-encoded-secret>
```

### 5. Full `.env` Snippet

```env
# ==============================
#   Encryption (required for OAuth token storage)
# ==============================
ENCRYPTION_KEY=<output of: openssl rand -hex 32>

# ==============================
#   Google OAuth
# ==============================
GOOGLE_CLIENT_ID=<client-id>.apps.googleusercontent.com
GOOGLE_CLIENT_SECRET=<client-secret>
GOOGLE_REDIRECT_URI=http://localhost:4000/api/auth/google/callback
# Reminder: no `GOOGLE_` prefix on this var (a typo'd `GOOGLE_OAUTH_SUCCESS_REDIRECT_URI` is silently ignored).
OAUTH_SUCCESS_REDIRECT_URI=http://localhost:3000/settings

# ==============================
#   Recall.ai
# ==============================
RECALL_AI_API_KEY=<recall-api-key>
RECALL_AI_REGION=us-east-1
RECALL_AI_WEBHOOK_SECRET=whsec_<signing-secret>
```

### 6. Development Flow

1. Generate and set `ENCRYPTION_KEY`.
2. Start the backend: `cargo run`.
3. Start an ngrok tunnel: `ngrok http 4000`.
4. Register the ngrok URL as the Recall.ai webhook endpoint (see above).
5. In the frontend, connect Google Meet via the settings page — this triggers the OAuth flow to `GOOGLE_REDIRECT_URI`.
6. Once connected, starting a coaching session with a Google Meet link will dispatch a Recall.ai bot. Bot events arrive at `/webhooks/recall_ai` and are verified using `RECALL_AI_WEBHOOK_SECRET`.

---

## Production Setup

### 1. Encryption Key

Generate a key on the production host and store it in your secrets manager (never commit it):

```bash
openssl rand -hex 32
```

Set `ENCRYPTION_KEY` in your environment or secret store. All existing encrypted tokens become unreadable if this key changes, so treat it as permanent once the service is live.

### 2. Google OAuth

Use the same Google Cloud project as development, or create a dedicated production project.

Key differences from local setup:

- **Redirect URI**: Set to your production domain, e.g. `https://api.myrefactor.com/api/auth/google/callback`
- **OAuth consent screen status**: Submit for Google verification to move out of "Testing" mode (required for non-test users to authorize)
- **Authorized redirect URIs**: Add your production URI in the OAuth 2.0 Client ID settings

```env
GOOGLE_CLIENT_ID=<client-id>.apps.googleusercontent.com
GOOGLE_CLIENT_SECRET=<client-secret>
GOOGLE_REDIRECT_URI=https://api.myrefactor.com/api/auth/google/callback
# Reminder: no `GOOGLE_` prefix on this var (a typo'd `GOOGLE_OAUTH_SUCCESS_REDIRECT_URI` is silently ignored).
OAUTH_SUCCESS_REDIRECT_URI=https://myrefactor.com/settings
```

### 3. Recall.ai

- **Webhook URL**: Set to your production endpoint, e.g. `https://api.myrefactor.com/webhooks/recall_ai`
- No tunnel required — the production host is directly reachable
- Use production API keys and signing secrets, not development ones

```env
RECALL_AI_API_KEY=<production-api-key>
RECALL_AI_REGION=us-east-1          # or eu-west-2
RECALL_AI_WEBHOOK_SECRET=whsec_<production-signing-secret>
```

### 4. Full Production Environment Variables

In addition to the variables in [README.md](../README.md) (database, Tiptap, Resend), add:

```env
# Encryption
ENCRYPTION_KEY=<64-hex-char key from secrets manager>

# Google OAuth
GOOGLE_CLIENT_ID=<client-id>.apps.googleusercontent.com
GOOGLE_CLIENT_SECRET=<client-secret>
GOOGLE_REDIRECT_URI=https://api.myrefactor.com/api/auth/google/callback
# Reminder: no `GOOGLE_` prefix on this var (a typo'd `GOOGLE_OAUTH_SUCCESS_REDIRECT_URI` is silently ignored).
OAUTH_SUCCESS_REDIRECT_URI=https://myrefactor.com/settings

# Recall.ai
RECALL_AI_API_KEY=<production-api-key>
RECALL_AI_REGION=us-east-1
RECALL_AI_WEBHOOK_SECRET=whsec_<production-signing-secret>
```

### 5. Deployment

See [docs/cicd/production-deployment.md](cicd/production-deployment.md) for the full deployment process.
