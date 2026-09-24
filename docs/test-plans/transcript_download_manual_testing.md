# Test Plan: Manually Testing Transcript Download

Verify `GET /coaching_sessions/{session_id}/transcriptions/{transcription_id}`
negotiates on `Accept`, filters by `speaker=coach|coachee`, and returns the
documented error slugs.

> [!IMPORTANT]
> The mock suite covers negotiation and error mapping but never a real
> `transcript_segments` table or a real browser. Cases 1 through 4 and 10 are the
> only proof the resolver matches real seeded names and that the filename
> reaches a cross-origin fetch.

## 1. Prerequisites

- Backend on branch `feat/transcript-download`, running on `:4000`, DB seeded
  (`cargo run --bin seed_db`).
- A coaching session whose relationship has a coach and a coachee, with one
  `completed` transcription and at least two speakers. Two ways to get one:
  1. Run a recorded meeting end to end through the Recall webhook (see
     `docs/implementation-plans/ai-meeting-milestone-2.md`).
  2. Insert rows directly (section 1.2). Fastest for local runs.
- Every request needs the `x-version` header, or `CompareApiVersion` rejects it
  with 400 before any handler runs.

### 1.1 Helpers

```sh
BASE=http://localhost:4000
VER='x-version: 1.0.0'

# Log in and keep a per-actor cookie jar. $1 = jar name, $2 = email, $3 = password.
login() { curl -s -c "/tmp/$1.jar" -X POST "$BASE/login" \
  -H 'content-type: application/x-www-form-urlencoded' \
  --data-urlencode "email=$2" --data-urlencode "password=$3" -o /dev/null -w '%{http_code}\n'; }

# Download. $1 = jar, $2 = Accept (or "" for none), $3 = query string (or "")
dl() { curl -s -i -b "/tmp/$1.jar" -H "$VER" ${2:+-H "accept: $2"} \
  "$BASE/coaching_sessions/$SESSION/transcriptions/$TRANSCRIPTION$3"; }

login coach james.hodapp@gmail.com password      # seeded coach, display_name "Jim H"
login coachee calebbourg2@gmail.com password     # seeded coachee
SESSION=<session id>
TRANSCRIPTION=<transcription id>
```

### 1.2 SQL fixture

Speaker labels must match how the seeded users would appear in a meeting:
the coach resolves via `display_name` (`Jim H`), the coachee via
`first_name last_name` (`Caleb Bourg`). `Guest` exercises `role: null`.

```sql
SET search_path TO refactor_platform;

INSERT INTO meeting_recordings (id, coaching_session_id, bot_id, status)
VALUES ('11111111-1111-1111-1111-111111111111', '<session id>', 'manual-bot', 'completed');

INSERT INTO transcriptions (id, coaching_session_id, meeting_recording_id, external_id, status, speaker_count)
VALUES ('22222222-2222-2222-2222-222222222222', '<session id>',
        '11111111-1111-1111-1111-111111111111', 'manual-transcript', 'completed', 3);

INSERT INTO transcript_segments (transcription_id, speaker_label, text, start_ms, end_ms) VALUES
  ('22222222-2222-2222-2222-222222222222', 'Jim H',       'Good morning.',        0,     1500),
  ('22222222-2222-2222-2222-222222222222', 'Caleb Bourg', 'Morning.',             4000,  5000),
  ('22222222-2222-2222-2222-222222222222', 'Guest',       'Hi both.',             9000,  10000),
  ('22222222-2222-2222-2222-222222222222', 'Jim H',       '   ',                  12000, 12100),
  ('22222222-2222-2222-2222-222222222222', 'Caleb Bourg', 'Let us start.',        59000, 61000),
  ('22222222-2222-2222-2222-222222222222', 'Jim H',       'Wrapping up.',         3735000, 3737000);
```

For case 8, add a second transcription on another session with `status = 'queued'`.

## 2. Cases

Each case: command, expected status, expected headers or body shape.

### Case 1: no `Accept` returns metadata JSON with `speakers`

```sh
dl coach "" ""
```

**Pass:** 200, JSON `ApiResponse` with the transcription fields plus
`speakers: [{label, role}]` in first-appearance order:
`Jim H` → `coach`, `Caleb Bourg` → `coachee`, `Guest` → `null`.

### Case 2: `text/plain` returns the file

```sh
dl coach text/plain ""
```

**Pass:** 200, `content-type: text/plain; charset=utf-8`,
`content-disposition: attachment; filename="transcript-<YYYY-MM-DD>.txt"` where
the date is the session's start in the coach's timezone (seeded users are UTC, so
it equals the stored date; set the coach to `America/Los_Angeles` and a session at
01:30 UTC to see it roll back a day). Body starts with the three-line header
block, a blank line, then `[0:00] Jim H: Good morning.`. The whitespace-only
segment at 12s is absent. The 59s line reads `[0:59]`.

### Case 3: `speaker=coach` filters and marks the filename

```sh
dl coach text/plain "?speaker=coach"
```

**Pass:** 200, filename ends `-filtered.txt`, header reads `Speakers: Jim H`,
no `Caleb Bourg` or `Guest` lines.

### Case 4: both roles excludes only the guest

```sh
dl coach text/plain "?speaker=coach&speaker=coachee"
```

**Pass:** 200, header `Speakers: Jim H, Caleb Bourg`, `Guest` lines absent,
filename ends `-filtered.txt`.

### Case 5: unknown enum value

```sh
dl coach text/plain "?speaker=bob"
```

**Pass:** 400 JSON, `error: "invalid_speaker"`, message lists `coach` and
`coachee`.

### Case 5b: role whose user matches no label

Rename the coachee so no label matches, then request `speaker=coachee`.

```sh
COACHEE_ID=<caleb's user id>
curl -s -X PUT -b /tmp/coachee.jar -H "$VER" -H 'content-type: application/json' \
  "$BASE/users/$COACHEE_ID" -d '{"display_name":"Nobody Here","first_name":"Nobody","last_name":"Here"}'
dl coach text/plain "?speaker=coachee"
```

**Pass:** 422 JSON, `error: "speaker_not_identified"`, message names `coachee`
and lists `Jim H`, `Caleb Bourg`, `Guest`. Case 1 rerun now shows
`Caleb Bourg` with `role: null`.

Restore the name afterwards with SQL. `PUT /users/{id}` treats a JSON `null`
as an omitted field, so it cannot clear `display_name`:

```sql
UPDATE refactor_platform.users
SET display_name = NULL, first_name = 'Caleb', last_name = 'Bourg'
WHERE id = '<caleb's user id>';
```

### Case 6: unsupported `Accept`

```sh
dl coach image/png ""
```

**Pass:** 406, plain-text body, no JSON envelope.

### Case 7: transcription from another session

```sh
OTHER=<transcription id belonging to a different session>
curl -s -i -b /tmp/coach.jar -H "$VER" \
  "$BASE/coaching_sessions/$SESSION/transcriptions/$OTHER"
```

**Pass:** 404 JSON, `error: "transcription_not_found"`.

### Case 8: transcription not completed

Point `SESSION` / `TRANSCRIPTION` at the `queued` fixture.

```sh
dl coach text/plain ""
dl coach "" ""
```

**Pass:** text → 409 JSON `error: "transcription_not_completed"`. JSON → 200
with `status: "queued"` and `speakers: []`.

### Case 9: non-participant

Log in as a user who is neither coach nor coachee on the session
(`dmanley@hotmail.com` in the seed).

```sh
login admin dmanley@hotmail.com password
dl admin text/plain ""
```

**Pass:** 403, plain-text `FORBIDDEN`. A SuperAdmin who is not a participant
also gets 403; the access check requires participation unconditionally.

### Case 10: browser can read `Content-Disposition`

From the frontend dev server, fetch the text representation in devtools:

```js
const r = await fetch(`${API}/coaching_sessions/${s}/transcriptions/${t}`,
  { credentials: 'include', headers: { accept: 'text/plain', 'x-version': '1.0.0' } });
r.headers.get('content-disposition');
```

**Pass:** returns the `attachment; filename="..."` value, not `null`. A `null`
means `CONTENT_DISPOSITION` is missing from `expose_headers` in `web/src/lib.rs`.

### Case 10b: OpenAPI surface

Open the RapiDoc page and `GET /api-docs/openapi2.json` (the spec is not served at `/openapi.json`).

**Pass:** `components.schemas.SpeakerRole` is a named schema with
`enum: ["coach","coachee"]` and a description (OpenAPI string enums carry one
description; the per-variant doc comments stay in code). `Speaker` and the
transcription-with-speakers schema are named and referenced by `$ref`, not
inlined. The `speaker` query param on the route references `SpeakerRole` and is
marked repeatable (`explode: true`, array type). The 200 response lists both
`application/json` and `text/plain`.

### Case 11: timestamp at or over one hour

The fixture already carries a segment at `start_ms = 3735000`. If using a real
recording that is shorter than an hour, bump one row:

```sql
UPDATE refactor_platform.transcript_segments SET start_ms = 3735000, end_ms = 3737000
WHERE id = '<segment id>';
```

**Pass:** the line renders as `[1:02:15] Jim H: Wrapping up.` and sorts last.

## 3. Cleanup

- Restore any bumped `transcript_segments` rows from case 11.
- Restore the coachee's name from case 5b if not already done.
- Delete the SQL fixture rows if they were inserted by hand
  (`meeting_recordings` cascades to `transcriptions` and `transcript_segments`):

```sql
DELETE FROM refactor_platform.meeting_recordings
WHERE id = '11111111-1111-1111-1111-111111111111';
```

## 4. Results

Run 2026-09-21 against the local backend with the section 1.2 fixture on the
seeded Jim / Caleb relationship.

| Case | Result | Date |
|---|---|---|
| 1 | PASS | 2026-09-21 |
| 2 | PASS | 2026-09-21 |
| 3 | PASS | 2026-09-21 |
| 4 | PASS | 2026-09-21 |
| 5 | PASS | 2026-09-21 |
| 5b | PASS | 2026-09-21 |
| 6 | PASS | 2026-09-21 |
| 7 | PASS | 2026-09-21 |
| 8 | PASS | 2026-09-21 |
| 9 | PASS | 2026-09-21 |
| 10 | SKIPPED (needs a browser against the frontend) | 2026-09-21 |
| 10b | PASS | 2026-09-21 |
| 11 | PASS | 2026-09-21 |

### Notes from the run

- Case 9: a non-participant SuperAdmin (`admin@refactorcoach.com`) also got
  403. The case text above was corrected; the first draft claimed 200.
- Case 5b: the restore originally used `PUT /users/{id}` with
  `"display_name": null`, which is a no-op. Corrected to SQL above.
- Case 10b: the first draft expected per-variant descriptions on
  `SpeakerRole`. utoipa emits one schema-level description for a string enum,
  which is all OpenAPI 3 allows. Corrected above.
- Case 8's `queued` fixture went on a second session in the same relationship
  and doubled as case 7's other-session transcription. Its
  `meeting_recordings.status` was `recording` (`in_progress` is not a valid
  value).
