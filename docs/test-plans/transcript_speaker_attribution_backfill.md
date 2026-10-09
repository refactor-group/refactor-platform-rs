# Runbook: Backfill Transcript Speaker Attribution (from a local dev machine)

Attribute transcripts recorded **before** speaker attribution shipped, using the same rules new
transcripts get at completion. Run from your laptop against production, dry run first.

## What it does and why

Transcripts completed before this feature have no `transcript_participants` rows. After it ships,
roles come only from those rows, so in older transcripts nobody shows as coach or coachee, and coach
or coachee filtered downloads return 422.

For each completed transcription with no participant rows, the backfill:

1. Re-downloads its transcript from Recall to recover the participant data (ids, host flags) we
   used to discard.
2. Rebuilds the segments in memory and compares them with the stored ones. **Only an exact match
   (same count, same text, same start times, in order) proceeds.** Anything else is skipped and
   reported.
3. Decides the coach and coachee with the attribution rules (meeting host, plus Google's attendee
   list when enabled and still available), then, in one transaction per transcription, inserts the
   participant rows and sets `participant_id` on the **existing** segments.

It never deletes or rewrites a segment, so the worst outcome is "not attributed". It is safe to
rerun: transcriptions that already have participant rows are skipped, and a transcription where
nobody was identified is left unlinked so a later run can retry it. That check happens inside each
transaction, so a transcript completing during the run is never touched twice.

## When

| Step | Allowed |
|---|---|
| Dry run against production | Any time the backfill is on your checkout, **even before #434 is deployed** (a missing `transcript_participants` table reads as "nothing attributed yet") |
| Apply against a fork | **Before** #434 deploys: fork production, then run #434's migrations on the fork |
| Apply against production | **Right after** #434 deploys, once the fork rehearsal checked out. Until it runs, older transcripts show no coach or coachee (an accepted gap), so keep it short |

## 1. Prerequisites

- A checkout of the branch containing the backfill (the `419-transcript-speaker-attribution` branch
  until it merges, `main` after). Run every command from that checkout's root.
- Rust toolchain (`cargo`).
- The production database CA certificate at
  `/Users/jhodapp/Projects/refactor-coaching/refactor-platform-rs/ca-certificate.crt`
  (already present; `*.crt` is gitignored).
- Your laptop's IP allowed by the DigitalOcean database's **Trusted Sources** (it was for the
  docs-collab import).
- SSH access to the production host (`deploy@ubuntu-droplet1`), only to read the Recall key.

## 2. Create `.env.backfill`

`.env*` is gitignored. Create it in the checkout root and fill the password from your password
manager or the DigitalOcean console. The URL **must** stay double-quoted, or `source` cuts it at `&`.

```sh
DATABASE_URL="postgresql://refactor:<PASSWORD>@db-postgresql-1-do-user-21553142-0.d.db.ondigitalocean.com:25060/refactor?sslmode=require&sslrootcert=/Users/jhodapp/Projects/refactor-coaching/refactor-platform-rs/ca-certificate.crt"
DATABASE_SCHEMA=refactor_platform
RECALL_AI_REGION=us-west-2
```

Google evidence is off by default, so no Google credentials or token decryption are needed for a
dry run.

Load the production Recall key into the current shell **without printing it** (it is read from the
production backend container's environment):

```sh
export RECALL_AI_API_KEY="$(ssh deploy@ubuntu-droplet1 \
  "docker inspect --format '{{range .Config.Env}}{{println .}}{{end}}' rust-app | sed -n 's/^RECALL_AI_API_KEY=//p'")"
[ -n "$RECALL_AI_API_KEY" ] && echo "Recall key loaded" || echo "Recall key MISSING"
```

Then load the file:

```sh
set -a; source .env.backfill; set +a
```

> [!NOTE]
> No other variables are required: every other setting the backend reads has a default or is
> optional, and the backfill does not send email or use collaboration services. Without
> `RECALL_AI_API_KEY` the binary exits before touching the database.

## 3. Dry run (read-only)

```sh
cargo run --bin backfill_transcript_speakers
```

Use a debug build (no `--release`): the backfill waits on Recall and the database, not the CPU, and
release builds of this workspace currently fail on some macOS toolchains (a `sqlx_macros` dylib
"mis-aligned LINKEDIT string pool" error).

Without `BACKFILL_APPLY=1` the database session is opened with `default_transaction_read_only=on`,
so Postgres rejects any write even if the code tried one.

It prints a summary and writes a CSV report, `backfill-report-<timestamp>.csv`, in the current
directory (no names or transcript text, just ids, outcomes, and counts):

| Column | Meaning |
|---|---|
| `transcription_id`, `coaching_session_id` | which transcript |
| `outcome` | `would_attribute` (dry run) or `attributed` (apply), `already_done`, `nobody_identified` (left unlinked so a later run can retry), `segments_differ`, `recall_missing`, `not_google_meet`, `error` |
| `speakers` | speakers Recall reports |
| `coach`, `coachee` | `yes` / `no`: would each be identified |
| `detail` | short reason for anything that is not `would_attribute` or `attributed` (never a name or text). For `segments_differ`: `segment count stored N rebuilt M`, `content differs at start_ms N`, or `ambiguous speakers at start_ms N` |

Optional:

- `BACKFILL_LIMIT=20` processes at most 20 transcriptions (good for a first look).
- `BACKFILL_SINCE=2026-09-01` only considers transcriptions created on or after that date.

**What to look at:** how many `would_attribute`, how many with `coach=yes`, and every
`segments_differ` / `error` row. Share the CSV with the overseer before applying anything.

## 4. Rehearse on a fork (before #434 deploys)

1. In the DigitalOcean console, open the production database cluster and **Fork** it (creates a new
   cluster from the latest backup). Note it costs money while it exists.
2. Add your IP to the fork's Trusted Sources; copy its connection string into a second file,
   `.env.backfill.fork`, using the same certificate path and schema lines as above.
3. Bring the fork's schema up to #434 (production does not have the new tables yet). From the
   checkout's root, with `sea-orm-cli` installed:
   ```sh
   set -a; source .env.backfill.fork; set +a
   sea-orm-cli migrate up -s refactor_platform
   ```
   It applies only #434's two migrations, as long as the checkout contains every migration production
   has already run (merge `main` into the branch first if `main` has gained migrations since).
4. Record the segment fingerprint on the fork (should be identical before and after):
   ```sql
   SET search_path TO refactor_platform;
   SELECT count(*), md5(string_agg(id::text || '|' || start_ms || '|' || text, ',' ORDER BY id))
   FROM transcript_segments;
   ```
5. Apply against the fork in two passes. Google evidence is apply-only, because looking it up
   writes refreshed tokens and account ids; the first pass needs this machine to have the
   backend's Google OAuth and token encryption settings.
   ```sh
   set -a; source .env.backfill.fork; set +a
   # Pass 1: recent transcripts get the Google cross-check while Google still has attendee lists.
   BACKFILL_APPLY=1 BACKFILL_GOOGLE=1 BACKFILL_SINCE=<today minus 30 days> \
     cargo run --bin backfill_transcript_speakers
   # Pass 2: everything else, without Google.
   BACKFILL_APPLY=1 cargo run --bin backfill_transcript_speakers
   ```
6. Verify on the fork:
   - the fingerprint from step 4 is **unchanged**;
   - the two apply CSVs' `attributed` counts together are close to the dry run's
     `would_attribute` count (Google can withhold a few where it disagrees with the host);
   - spot-check two attributed transcriptions with the attribution query in
     `transcript_speaker_attribution_manual_testing.md` section 1.4;
   - rerunning the dry run against the fork lists only transcriptions that were not linked
     (`nobody_identified`, `segments_differ`, `recall_missing`, `not_google_meet`, `error`) and
     none `would_attribute`; attributed ones are no longer candidates.
7. Destroy the fork.

## 5. Apply to production

1. Note the current UTC time (for point-in-time recovery if ever needed) and record the segment
   fingerprint from step 4.4 against production.
2. Apply in the same two passes as the rehearsal (Google is apply-only):
   ```sh
   set -a; source .env.backfill; set +a
   BACKFILL_APPLY=1 BACKFILL_GOOGLE=1 BACKFILL_SINCE=<today minus 30 days> \
     cargo run --bin backfill_transcript_speakers
   BACKFILL_APPLY=1 cargo run --bin backfill_transcript_speakers
   ```
3. Re-check the fingerprint (unchanged) and rerun the dry run: it lists only transcriptions that
   were not linked (`nobody_identified`, `segments_differ`, `recall_missing`, `not_google_meet`,
   `error`) and none `would_attribute`.
4. Open one backfilled session in the app: the transcript shows profile names for the coach and
   coachee, and the coach-only download works.
5. Delete `.env.backfill` and run `unset RECALL_AI_API_KEY`.

## Undo

The backfill only inserts participant rows and fills `participant_id`; it never changes text. To undo
it for the transcriptions it applied (their ids are the `attributed` rows of the apply runs'
CSVs):

```sql
SET search_path TO refactor_platform;
BEGIN;
UPDATE transcript_segments SET participant_id = NULL WHERE transcription_id IN (<ids>);
DELETE FROM transcript_participants WHERE transcription_id IN (<ids>);
COMMIT;
```

## Troubleshooting

- **Connection timeout:** the `sslrootcert` path is wrong or unquoted, or your IP is not in Trusted
  Sources.
- **Many `recall_missing`:** Recall no longer holds those transcripts; nothing can be done for them.
- **`segments_differ`:** the rebuild must contain exactly the stored lines (same start times and text).
  Lines that start in the same millisecond are matched by text, so their stored order does not
  matter. `ambiguous speakers at start_ms N` means two different people said the identical words in
  the same millisecond, so who said which cannot be known and the transcript is left alone. Many
  `segments_differ` rows: stop and share the CSV.
- **`not_google_meet`:** Zoom (deferred) or a session without a platform-created Meet link.
