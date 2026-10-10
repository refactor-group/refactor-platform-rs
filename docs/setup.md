# Platform Setup Guide

This guide covers setting up the Refactor platform for local development and production deployment.

---

## Development Setup

### Prerequisites

- Rust toolchain (`rustup` + stable)
- PostgreSQL 14+ (see [README.md](../README.md) for DB setup)
- `cargo`, `sea-orm-cli` 2.0.3 (matches `sea-orm-migration`)
- [ngrok](https://ngrok.com/) with a reserved domain (for Recall.ai webhooks; see section 4)

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
   - Authorized redirect URI: `http://localhost:4000/oauth/google/callback`
   - Download or copy the Client ID and Client Secret

#### Environment Variables

```env
GOOGLE_CLIENT_ID=<client-id>.apps.googleusercontent.com
GOOGLE_CLIENT_SECRET=<client-secret>
GOOGLE_REDIRECT_URI=http://localhost:4000/oauth/google/callback

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

Recall.ai sends recording and transcript events to `POST /webhooks/recall_ai`, so during local development it has to reach your machine. Recall has no API for webhook endpoints, and every endpoint in a workspace receives every event in that workspace. So each developer uses **their own Recall development workspace** with one endpoint pointing at their own reserved ngrok domain. Set it up once; nothing needs editing afterwards.

#### One-time setup

1. **Recall workspace.** In the Recall dashboard, create a development workspace for yourself (never the production one; a shared one would also deliver other developers' bot events to you). Note its region (`us-west-2` for ours).
2. **API key and signing secret.** In that workspace, create an API key, then under **Developers > API Keys & Secrets** create the workspace secret (starts with `whsec_`).
3. **ngrok domain.** Install [ngrok](https://ngrok.com/), run `ngrok config add-authtoken <token>`, and reserve a static domain in the ngrok dashboard (one is included on the free plan).
4. **`.env`:**

   ```env
   NGROK_DOMAIN=<your-reserved-domain>        # e.g. jim-dev.ngrok.app
   RECALL_AI_API_KEY=<key from your workspace>
   RECALL_AI_REGION=us-west-2                 # your workspace's region
   RECALL_AI_WEBHOOK_SECRET=whsec_<your workspace secret>
   ```

5. **Open the tunnel once** with the backend running (see Development Flow below):

   ```bash
   scripts/recall_webhook_tunnel.sh
   ```

   It prints your endpoint URL and the events to subscribe to.
6. **Recall endpoint.** In your workspace, **Webhooks > Add Endpoint**: paste the URL the script printed (`https://<your-domain>/webhooks/recall_ai`) and subscribe to every event it lists. The `/webhooks/recall_ai` path is required: the bare host answers POSTs with 405, because unmatched paths fall through to a static-file handler.

#### Every session

Start the backend, then run `scripts/recall_webhook_tunnel.sh` in another terminal and leave it running; Ctrl-C stops the tunnel. Before opening the tunnel it checks:

- `NGROK_DOMAIN`, `RECALL_AI_REGION`, and the `whsec_` form of `RECALL_AI_WEBHOOK_SECRET`;
- that Recall accepts your key in that region (a key from another region or workspace is rejected here, rather than surfacing later as transcripts that never arrive);
- that the backend answers on `http://localhost:$PORT/health` (`PORT` defaults to 4000);
- that `GOOGLE_REDIRECT_URI` points at the local backend (a warning only).

Once the tunnel is up it sends an unsigned `POST /webhooks/recall_ai` through it. A **401** from the handler proves the route reaches the backend; anything else is reported with what to fix. Because that check is unsigned, the backend logs one `Svix validation error ... Missing svix-id/webhook-id header` warning each time the tunnel opens. That warning is expected; real Recall deliveries always carry the signature headers. `scripts/recall_webhook_tunnel.sh --check` does all of this and then closes the tunnel, which is handy for verifying a setup.

If ngrok reports the domain is already online, another ngrok (often a forgotten earlier run) holds it; stop that one first.

### 5. Coaching Note Images (Object Storage)

Images pasted into a coaching note are uploaded to object storage; the note itself stores only an
image id and resolves the URL at render time. `OBJECT_STORE_BACKEND` picks the backend.

Each upload is spooled through the system temp directory (`TMPDIR`, else `/tmp`) and streamed to
storage from there, so the server's memory does not grow with image size. Keep that directory on
disk: mounting it as `tmpfs` would put every in-flight upload back in RAM.

**Local development needs no DigitalOcean Spaces account.** The default `local` backend writes to
`OBJECT_STORE_LOCAL_PATH`, a gitignored directory under the repo, and the read endpoint streams the
bytes back instead of redirecting to a presigned URL.

```env
OBJECT_STORE_BACKEND=local
OBJECT_STORE_LOCAL_PATH=./.local-object-store
COACHING_SESSION_IMAGE_MAX_BYTES=10485760          # 10 MB upload cap
COACHING_SESSION_IMAGE_PRESIGN_TTL_SECONDS=900     # lifetime of a presigned image GET URL
COACHING_SESSION_IMAGE_GRACE_PERIOD_HOURS=168      # how long a removed image survives undo
COACHING_SESSION_IMAGE_PURGE_POLL_MINUTES=60       # how often the purge job sweeps
```

#### Using DigitalOcean Spaces

Only needed to exercise the production path, and required in PR previews and production.

1. Create a Space at [cloud.digitalocean.com/spaces](https://cloud.digitalocean.com/spaces) and note
   its region (e.g. `nyc3`) and name.
2. Under **Settings → Spaces Keys**, generate an access key pair. The secret is shown once — store it
   in your secrets manager.
3. Set `OBJECT_STORE_BACKEND=spaces` plus the variables below. The regional endpoint must match
   `SPACES_REGION`.

```env
OBJECT_STORE_BACKEND=spaces
SPACES_ENDPOINT=https://nyc3.digitaloceanspaces.com
SPACES_REGION=nyc3
SPACES_BUCKET=<space-name>
SPACES_ACCESS_KEY_ID=<access-key-id>
SPACES_SECRET_ACCESS_KEY=<secret-access-key>
```

If the backend is `spaces` but any of these is missing, the app still boots — it logs a warning and
the image endpoints return 503 rather than failing startup.

To verify the whole pipeline end to end, follow
[docs/test-plans/coaching_session_images_manual_testing.md](test-plans/coaching_session_images_manual_testing.md).

### 6. Full `.env` Snippet

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
GOOGLE_REDIRECT_URI=http://localhost:4000/oauth/google/callback
OAUTH_SUCCESS_REDIRECT_URI=http://localhost:3000/settings

# ==============================
#   Recall.ai
# ==============================
NGROK_DOMAIN=<your-reserved-ngrok-domain>
RECALL_AI_API_KEY=<key from your own Recall development workspace>
RECALL_AI_REGION=us-west-2
RECALL_AI_WEBHOOK_SECRET=whsec_<your workspace signing secret>

# ==============================
#   Coaching note images
# ==============================
OBJECT_STORE_BACKEND=local
OBJECT_STORE_LOCAL_PATH=./.local-object-store
COACHING_SESSION_IMAGE_MAX_BYTES=10485760
COACHING_SESSION_IMAGE_PRESIGN_TTL_SECONDS=900
COACHING_SESSION_IMAGE_GRACE_PERIOD_HOURS=168
COACHING_SESSION_IMAGE_PURGE_POLL_MINUTES=60

# ==============================
#   Collaborative notes (docs-collab-server)
# ==============================
TIPTAP_URL=http://localhost:1234
TIPTAP_AUTH_KEY=<any shared secret; the launcher passes it to both servers>
TIPTAP_JWT_SIGNING_KEY=<any shared secret; the launcher passes it to both servers>
```

### 7. Collaborative Notes Server (docs-collab-server)

Coaching-session notes sync through the self-hosted `docs-collab-server`, not TipTap Cloud. The frontend has no Cloud fallback: if its collab URL is set but nothing is listening, the editor opens local-only with **no error**, and notes silently never sync. So the server has to be running whenever you work on notes.

`scripts/run_backend.sh` builds and starts it alongside the app server, deriving everything from `.env`:

- `JWT_SIGNING_KEY` and `MANAGEMENT_AUTH_KEY` come from `TIPTAP_JWT_SIGNING_KEY` and `TIPTAP_AUTH_KEY`, so they match the app by construction.
- It uses its own local database, `refactor_collab`, built from the `POSTGRES_*` values and created on first run. This mirrors production and PR preview, and means notes survive `scripts/rebuild_db.sh`. Override with `COLLAB_DATABASE_URL` if you want it elsewhere.
- It binds `127.0.0.1:1234` (override with `COLLAB_BIND_ADDR`). If you change the bind address or port, the app-facing URL follows it (`http://localhost:<port>`, or set `COLLAB_URL` explicitly), and the frontend's `NEXT_PUBLIC_DOCS_COLLAB_URL` must be changed to match by hand; nothing validates that side.
- It exits with the status of the first binary that dies, so a startup failure such as a port already in use is reported as a failure, not a clean exit.

One-time `.env` change so the app talks to the local server instead of Cloud:

```env
TIPTAP_URL=http://localhost:1234
```

The script warns at startup if this is still pointing at Cloud. On the frontend side, `.env.local` needs `NEXT_PUBLIC_DOCS_COLLAB_URL="ws://localhost:1234"`.

Sanity check once it's up:

```bash
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:1234/health   # 200
```

If notes open but never sync or show presence, check that the collab server is running before anything else.

### 8. Development Flow

1. Generate and set `ENCRYPTION_KEY`.
2. Start the backend: `scripts/run_backend.sh` (app server plus collab server).
3. Open the Recall webhook tunnel: `scripts/recall_webhook_tunnel.sh` (one-time Recall endpoint setup in section 4).
4. Start the frontend.
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
OAUTH_SUCCESS_REDIRECT_URI=https://myrefactor.com/settings

# Recall.ai
RECALL_AI_API_KEY=<production-api-key>
RECALL_AI_REGION=us-east-1
RECALL_AI_WEBHOOK_SECRET=whsec_<production-signing-secret>

# Coaching note images (see "Coaching Note Images" under Development Setup)
OBJECT_STORE_BACKEND=spaces
SPACES_ENDPOINT=https://nyc3.digitaloceanspaces.com
SPACES_REGION=nyc3
SPACES_BUCKET=<space-name>
SPACES_ACCESS_KEY_ID=<access-key-id from secrets manager>
SPACES_SECRET_ACCESS_KEY=<secret-access-key from secrets manager>
COACHING_SESSION_IMAGE_MAX_BYTES=10485760
COACHING_SESSION_IMAGE_PRESIGN_TTL_SECONDS=900
COACHING_SESSION_IMAGE_GRACE_PERIOD_HOURS=168
COACHING_SESSION_IMAGE_PURGE_POLL_MINUTES=60
```

### 5. Deployment

See [docs/cicd/production-deployment.md](cicd/production-deployment.md) for the full deployment process.
