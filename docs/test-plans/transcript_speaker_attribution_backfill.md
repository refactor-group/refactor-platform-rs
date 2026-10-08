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
rerun: transcriptions that already have participant rows are skipped. That check happens inside each
transaction, so a transcript completing during the run is never touched twice.

## When

| Step | Allowed |
|---|---|
| Dry run against production | Any time the backfill is on your checkout, **even before #434 is deployed** (a missing `transcript_participants` table reads as "nothing attributed yet") |
| Apply against a fork | After #434 is deployed (the fork must contain the new tables) |
| Apply against production | After the fork rehearsal checks out |

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
# Google's attendee lists expire after 30 days, so they rarely help old transcripts.
# Off means no Google credentials or token decryption are needed on this machine.
BACKFILL_GOOGLE=0
```

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
> If the binary exits naming another required variable, add it to `.env.backfill` with a harmless
> value (the backfill does not send email or use collaboration services). The exact list is
> finalized when the binary lands; update this section then.

## 3. Dry run (read-only)

```sh
cargo run --release --bin backfill_transcript_speakers
```

Without `BACKFILL_APPLY=1` the database session is opened with `default_transaction_read_only=on`,
so Postgres rejects any write even if the code tried one.

It prints a summary and writes a CSV report, `backfill-report-<timestamp>.csv`, in the current
directory (no names or transcript text, just ids, outcomes, and counts):

| Column | Meaning |
|---|---|
| `transcription_id`, `coaching_session_id` | which transcript |
| `outcome` | `would_attribute`, `already_done`, `segments_differ`, `recall_missing`, `not_google_meet`, `error` |
| `speakers` | speakers Recall reports |
| `coach`, `coachee` | `yes` / `no`: would each be identified |
| `detail` | short reason for anything that is not `would_attribute` (never a name or text) |

Optional:

- `BACKFILL_LIMIT=20` processes at most 20 transcriptions (good for a first look).
- `BACKFILL_SINCE=2026-09-01` only considers transcriptions created on or after that date.

**What to look at:** how many `would_attribute`, how many with `coach=yes`, and every
`segments_differ` / `error` row. Share the CSV with the overseer before applying anything.

## 4. Rehearse on a fork (after #434 is deployed)

1. In the DigitalOcean console, open the production database cluster and **Fork** it (creates a new
   cluster from the latest backup). Note it costs money while it exists.
2. Add your IP to the fork's Trusted Sources; copy its connection string into a second file,
   `.env.backfill.fork`, using the same certificate path and schema lines as above.
3. Record the segment fingerprint on the fork (should be identical before and after):
   ```sql
   SET search_path TO refactor_platform;
   SELECT count(*), md5(string_agg(id::text || '|' || start_ms || '|' || text, ',' ORDER BY id))
   FROM transcript_segments;
   ```
4. Apply against the fork:
   ```sh
   set -a; source .env.backfill.fork; set +a
   BACKFILL_APPLY=1 cargo run --release --bin backfill_transcript_speakers
   ```
5. Verify on the fork:
   - the fingerprint from step 3 is **unchanged**;
   - `SELECT outcome, count(*)` from the CSV matches the dry run's `would_attribute` count;
   - spot-check two attributed transcriptions with the attribution query in
     `transcript_speaker_attribution_manual_testing.md` section 1.4;
   - rerunning the dry run against the fork reports `already_done` for them and nothing new.
6. Destroy the fork.

## 5. Apply to production

1. Note the current UTC time (for point-in-time recovery if ever needed) and record the segment
   fingerprint from step 4.3 against production.
2. ```sh
   set -a; source .env.backfill; set +a
   BACKFILL_APPLY=1 cargo run --release --bin backfill_transcript_speakers
   ```
3. Re-check the fingerprint (unchanged) and rerun the dry run: everything reports `already_done`,
   `segments_differ`, `recall_missing`, or `not_google_meet`, and nothing `would_attribute`.
4. Open one backfilled session in the app: the transcript shows profile names for the coach and
   coachee, and the coach-only download works.
5. Delete `.env.backfill` and run `unset RECALL_AI_API_KEY`.

## Undo

The backfill only inserts participant rows and fills `participant_id`; it never changes text. To undo
it for the transcriptions it applied (their ids are the `would_attribute` rows of the apply run's
CSV):

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
- **Many `segments_differ`:** stop and share the CSV; the rebuild disagrees with stored data and the
  rule is to leave those alone.
- **`not_google_meet`:** Zoom (deferred) or a session without a platform-created Meet link.
