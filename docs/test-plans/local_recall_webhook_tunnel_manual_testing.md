# Manual Test Plan: Local Recall.ai Webhooks

Verifies that a developer can record a real Google Meet session against their local backend and see
the transcript complete, using `scripts/recall_webhook_tunnel.sh` and their own Recall development
workspace (`docs/setup.md` section 4). Written for a human or an AI agent with a browser.

## Prerequisites

- The one-time setup in `docs/setup.md` section 4 is done: your own Recall development workspace,
  its API key and `whsec_` secret in `.env`, `NGROK_DOMAIN` set, and the workspace's webhook
  endpoint subscribed to every event the script lists.
- Google OAuth configured for local (`GOOGLE_REDIRECT_URI=http://localhost:4000/oauth/google/callback`).
- Local Postgres seeded (`cargo run --bin seed_db`), the frontend running, and a coach and coachee
  account in one coaching relationship that you can sign in as.
- A second Google account (or a second person) to join the meeting as the coachee.

## Part A: the script's checks

### A1. Happy path
1. Start the backend: `scripts/run_backend.sh`.
2. In another terminal: `scripts/recall_webhook_tunnel.sh --check`.

**Pass:** it reports the Recall key works in your region, the backend is up, and
`https://<NGROK_DOMAIN>/webhooks/recall_ai reaches the backend`; prints the dashboard setup with 11
events; exits 0; no `ngrok` process is left (`pgrep ngrok` prints nothing).

### A2. Backend down
1. Stop the backend. Run `scripts/recall_webhook_tunnel.sh --check`.

**Pass:** exits 1 naming `http://localhost:4000/health` and `scripts/run_backend.sh`; no tunnel opened.

### A3. Wrong region
1. Run `RECALL_AI_REGION=us-east-1 scripts/recall_webhook_tunnel.sh --check` (assuming your
   workspace is in `us-west-2`).

**Pass:** exits 1 saying Recall rejected the key in `us-east-1`; no tunnel opened.

### A4. Domain already online
1. Start the backend. In one terminal run `scripts/recall_webhook_tunnel.sh` and leave it up.
2. In a second terminal run `scripts/recall_webhook_tunnel.sh --check`.

**Pass:** the second run exits 1 with ngrok's output and the hint to stop the other ngrok; the first
tunnel keeps working (rerun A1 after stopping the second).

### A5. Ctrl-C
1. Run `scripts/recall_webhook_tunnel.sh`, wait for `Tunnel is up`, press Ctrl-C.

**Pass:** the script exits within a second or two and `pgrep ngrok` prints nothing.

## Part B: a real recording, end to end

### B1. Record and transcribe
1. Backend running; `scripts/recall_webhook_tunnel.sh` running.
2. Sign in as the coach. In Settings, connect Google.
3. Create a coaching session with a Google Meet link for the relationship; open it.
4. Join the Meet as the coach (signed in to the connected Google account); have the coachee join.
5. Start recording from the session page. Admit the bot if asked.
6. Talk for a minute (both people), then stop recording and leave the meeting.

**Pass:**
- the tunnel terminal stays up; the backend log shows `bot.joining_call`, `bot.in_call_recording`,
  `bot.done`, `recording.done`, `transcript.processing`, then `transcript.done` arriving at
  `/webhooks/recall_ai` with no signature errors;
- the session page's recording status moves through joining, recording, processing, completed;
- the transcript appears in the session's transcript panel without a page reload.

### B2. Speakers attributed
1. Open the transcript from B1.

**Pass:** lines show the coach's and coachee's profile names; filtering to the coach downloads
`transcript-<date>-<coach name>.txt` with only the coach's lines.

## Part C: isolation

### C1. Production is untouched
1. After B1, note the bot id from the backend log.
2. Check the production backend's log for that bot id (an operator with access).

**Pass:** production never received an event for it.

## Results

| Case | Result | Date | Notes |
|---|---|---|---|
| A1 | PASS | 2026-10-10 | Real ngrok domain; route verified in ~4s; no ngrok left |
| A2 | PASS | 2026-10-10 | Real Recall key; stopped before the tunnel |
| A3 | PASS | 2026-10-10 | Recall returned 401 for us-east-1 |
| A4 | PASS | 2026-10-10 | ERR_NGROK_334 shown with the hint; first tunnel kept answering 401 |
| A5 | PASS | 2026-10-10 | Ctrl-C in a terminal: exit 130 at once, no ngrok left |
| B1 | PASS | 2026-10-10 | Recording and transcript completed locally via the tunnel (24 words) |
| B2 | PASS | 2026-10-10 | Coach by account (host), coachee by elimination; 5 of 5 lines linked. Download not rechecked |
| C1 | PASS | 2026-10-10 | Production backend logs had no mention of the local bot id |
