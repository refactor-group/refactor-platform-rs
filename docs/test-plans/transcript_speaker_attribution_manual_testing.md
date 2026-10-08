# Test Plan: Manually Testing Transcript Speaker Attribution

Verify that when a recorded coaching session's transcript completes, each segment
spoken by the session's coach or coachee is attributed to that user from an
identity source (the coach's connected meeting account, then elimination for the
coachee), never from profile-name matching, and that nothing is attributed to the
wrong person.

> [!IMPORTANT]
> The unit and SQLite suites cover the resolution rules against recorded payloads.
> Part A is the only proof that real Zoom and Google Meet meetings, run through
> Recall.ai, carry the identifiers those rules depend on. Part B can be run by a
> person or an AI agent against a local backend with no meeting at all.

## What "correct" means

| Situation | Coach | Coachee | Anyone else |
|---|---|---|---|
| Coach joins with their connected account, one other person speaks | `account` | `elimination` | n/a |
| Coach joins with their connected account, two or more others speak | `account` | `null` | `null` |
| Coach joins signed out or with another account | `null` | `null` | `null` |
| Coach is the only speaker | `account` | n/a | n/a |
| Coachee absent, a stand-in speaks alone with the coach | `account` | stand-in labeled coachee (accepted limitation) | n/a |

`account` and `elimination` are the values of `transcript_participants.match_source`.
Profile names never decide anything: a coach who joins as "J. Hodapp" with profile
name "Jim Hodapp" must still be attributed.

**What people see.** An attributed speaker is shown by their platform profile name, whatever
name they joined the meeting with. An unattributed speaker is shown by the name they typed when
joining, as before; one with no name (a dial-in) is shown as `Guest 1`, `Guest 2`, ... in order
of first speaking. Labels are unique within a transcript: if an unattributed speaker's typed name
equals a label already in use, it gets a suffix (`Jim H (2)`). A person who rejoins keeps one
label. This holds in all three places a transcript is read: the JSON `speakers` list, the
plain-text download, and the transcript segments the session page displays, and the three always
agree.

## 1. Prerequisites

### 1.1 Environments

- **Part A** needs Google OAuth and inbound Recall.ai webhooks. PR previews support neither yet
  (tracked in #435), so run Part A in production after #434 deploys, as post-deploy verification.
  The attribution rules were also replayed against two real production recordings before merge.
- **Part B** needs only a local backend on `:4000` from branch
  `419-transcript-speaker-attribution`, migrations applied, DB seeded
  (`cargo run --bin seed_db`).

### 1.2 People and accounts (Part A)

- **Coach:** a platform user who has connected Zoom and Google in Settings.
  Note the exact account they connected with.
- **Coachee:** a platform user in a coaching relationship with the coach. Does not
  need any connected account.
- **Guest:** any third person (or a second browser profile) for the guest scenarios.
- Before each scenario, give the coach a meeting display name that differs from their
  platform profile name (for example "J. Hodapp" vs "Jim Hodapp"), so a pass cannot
  come from name matching.

### 1.3 Helpers

```sh
BASE=http://localhost:4000          # or the PR preview API base, e.g. https://<host>/pr-<NUM>/api
VER='x-version: 1.0.0'

# $1 = jar name, $2 = email, $3 = password
login() { curl -s -c "/tmp/$1.jar" -X POST "$BASE/login" \
  -H 'content-type: application/x-www-form-urlencoded' \
  --data-urlencode "email=$2" --data-urlencode "password=$3" -o /dev/null -w '%{http_code}\n'; }

# Transcription metadata (JSON with speakers). $1 = jar
meta() { curl -s -b "/tmp/$1.jar" -H "$VER" \
  "$BASE/coaching_sessions/$SESSION/transcriptions/$TRANSCRIPTION"; }

# Plain-text download. $1 = jar, $2 = query string (or "")
txt() { curl -s -i -b "/tmp/$1.jar" -H "$VER" -H 'accept: text/plain' \
  "$BASE/coaching_sessions/$SESSION/transcriptions/$TRANSCRIPTION$2"; }

# Session-level latest transcription. $1 = jar
latest() { curl -s -b "/tmp/$1.jar" -H "$VER" "$BASE/coaching_sessions/$SESSION/transcriptions"; }

# Segments as the session page reads them. $1 = jar
segs() { curl -s -b "/tmp/$1.jar" -H "$VER" \
  "$BASE/coaching_sessions/$SESSION/transcriptions/$TRANSCRIPTION/transcription_segments"; }
```

### 1.4 Attribution query

Run against the environment's database after each Part A scenario. This is the
ground truth every scenario checks.

```sql
SET search_path TO refactor_platform;

SELECT p.display_name, p.is_host, p.platform_account_id, p.match_source,
       u.email AS attributed_to,
       count(s.id) AS segments
FROM transcript_participants p
LEFT JOIN users u ON u.id = p.user_id
LEFT JOIN transcript_segments s ON s.participant_id = p.id
WHERE p.transcription_id = '<transcription id>'
GROUP BY p.id, u.email
ORDER BY min(s.start_ms);

-- Every segment has a participant (expect 0).
SELECT count(*) FROM transcript_segments
WHERE transcription_id = '<transcription id>' AND participant_id IS NULL;
```

## Part A: real meetings (human run)

Each scenario: schedule a session in the platform UI with the named provider (so the
meeting is created through the coach's connection), start recording, talk for at least
two minutes with each listed speaker saying several sentences, end the meeting, and wait
for the transcript to finish (the session page shows the transcript). Then run 1.4 and
the listed API checks.

### A1. Zoom, coach under their connected account, coachee speaks

> [!NOTE]
> Blocked until Zoom works and phase Z lands. Until then, a Zoom session should behave like A3:
> typed names, no roles.

1. Coach joins Zoom signed in to the connected Zoom account, display name "J. Hodapp".
2. Coachee joins under any name and speaks.

**Pass:**
- 1.4 shows the coach's row `match_source = account`, attributed to the coach, with a
  non-null `platform_account_id`; the coachee's row `elimination`, attributed to the coachee.
- `meta coach` lists the coach's and coachee's profile names with roles `coach` and `coachee`.
  "J. Hodapp" appears nowhere.
- `txt coach "?speaker=coach"` is 200, filename ends `-filtered.txt`, every line is the
  coach's, and the header and lines use the coach's profile name.
- The session page's transcript shows the same two profile names.

### A2. Google Meet, coach under their connected account, coachee speaks

1. Coach joins Meet signed in to the Google account they connected, display name "J. Hodapp".
2. Coachee joins under any name and speaks.

**Pass:** same checks as A1. The coach's row has `platform_account_id` null (Meet has no stable
Recall id); attribution came from the Meet conference record, so `match_source = account`.

### A3. Coach joins signed out (or with a different account)

1. On Meet, the coach joins from a private window signed out (guest), or signed in to a
   different Google account. On Zoom, join from the web client without signing in.
2. Coachee joins and speaks.

**Pass:**
- 1.4 shows every row with `match_source` and `attributed_to` null.
- `meta coach`, `txt coach ""`, `segs coach`, and the session page show both people by the names
  they typed when joining, each with `role: null` in `meta`.
- `txt coach "?speaker=coach"` is 422 `speaker_not_identified`.
- `txt coach ""` (no filter) is 200 and still contains everyone's lines.

### A4. A guest speaks too

1. Coach (connected account), coachee, and a guest all join and all speak.

**Pass:** coach `account`; coachee and guest both null. The coach shows by profile name; the
coachee and guest show by their typed names with `role: null`. Download with
`?speaker=coachee` is 422. This is correct: with two other speakers we cannot tell which is the
coachee.

### A5. A guest is present but silent

1. Coach, coachee, and a guest join. Only the coach and coachee speak.

**Pass:** same as A1. A silent participant has no segments, so it does not block elimination.

### A6. Coachee absent, stand-in speaks (accepted limitation)

1. Coach (connected account) and a stand-in join; the coachee does not.
2. The stand-in speaks.

**Pass (documents the accepted behavior):** the stand-in's row is attributed to the
coachee with `match_source = elimination`, so the stand-in's lines show under the coachee's
profile name. Record the result; it is not a failure.

### A7. Coachee drops and rejoins (Meet)

1. A2 setup. Midway, the coachee leaves and rejoins, then speaks again.

**Pass:** 1.4 shows two coachee participant rows (Recall creates a new participant on a
Meet rejoin), both attributed to the coachee by `elimination`. `?speaker=coachee` returns
lines from before and after the rejoin.

### A8. Coach is the only speaker

1. Coach speaks; coachee joins muted, or not at all.

**Pass:** coach `account`; no coachee attribution. `?speaker=coachee` is 422,
`?speaker=coach` is 200.

### A9. Coach and coachee use the same display name

1. Both join as "Jim". Coach under the connected account.

**Pass:** 1.4 attributes both rows correctly (coach `account`, coachee `elimination`).
`?speaker=coach` and `?speaker=coachee` each return only that person's lines. Everywhere the
transcript is shown, the two people appear under their own profile names, not "Jim".

### A10. Google connection made before this change

Precondition: the coach's Google `oauth_connections.external_account_id` is null (true for
every Google connection made before this branch). Check with:

```sql
SELECT provider, external_account_id FROM refactor_platform.oauth_connections
WHERE user_id = '<coach user id>';
```

1. Run A2 without reconnecting Google.

**Pass:** after the transcript completes, the coach's Google row has a non-null
`external_account_id`, and attribution matches A2.

### A11. Identity lookup fails (coach disconnected)

1. Run A2, but disconnect Google in the coach's Settings after the meeting ends and before
   the transcript completes.

**Pass:** the transcription still reaches `completed` and segments exist; every row is
unattributed; backend logs one WARN naming the session id. No 500s, no failed transcription.

### A12. Session-level read carries speakers

After any of A1 to A9:

```sh
latest coach
```

**Pass:** the transcription JSON includes `speakers` matching what `meta coach` returns for
the same transcription.

### A13. Access control is unchanged

**Pass:** a logged-in user outside the relationship gets 403 from `meta`, `txt`, `segs`, and `latest`.

### A14. A guest who joins under the coach's name

1. Coach (connected account, any meeting name) and coachee join. A guest joins with the coach's
   exact profile name as their meeting name. All three speak.

**Pass:**
- 1.4: coach `account`; the guest and coachee null.
- The coach shows as their profile name; the guest shows as `<coach profile name> (2)` with
  `role: null`. The two never merge: the session page shows them in different colors, and the
  speaker filter lists them separately.
- Selecting the coach in the session page's filter enables the download; selecting the guest
  shows "Switch to All to download".

### A15. Dial-in without a name

1. Coach (connected account) and coachee join; a third person dials in by phone and speaks.

**Pass:** the dial-in shows as `Guest 1` (not a numeric id or `Unknown`). Coachee is null (two
other speakers), shown by typed name.

## Part B: local, no meeting (person or AI agent)

These insert attribution rows directly and prove the read side trusts stored attribution,
not names. Seeded coach: `james.hodapp@gmail.com` / `password`. Seeded coachee:
`calebbourg2@gmail.com` / `password`.

### B.0 Fixture

Pick a session whose relationship has the seeded coach and coachee, then:

```sql
SET search_path TO refactor_platform;

INSERT INTO meeting_recordings (id, coaching_session_id, bot_id, status)
VALUES ('41900000-0000-0000-0000-000000000001', '<session id>', 'manual-bot-419', 'completed');

INSERT INTO transcriptions (id, coaching_session_id, meeting_recording_id, external_id, status, speaker_count)
VALUES ('41900000-0000-0000-0000-000000000002', '<session id>',
        '41900000-0000-0000-0000-000000000001', 'manual-transcript-419', 'completed', 3);

-- Labels deliberately differ from both users' profile names.
INSERT INTO transcript_participants
  (id, transcription_id, provider_participant_id, display_name, user_id, match_source) VALUES
  ('41900000-0000-0000-0000-0000000000a1', '41900000-0000-0000-0000-000000000002', '100', 'J. Hodapp',
     '<coach user id>', 'account'),
  ('41900000-0000-0000-0000-0000000000a2', '41900000-0000-0000-0000-000000000002', '200', 'CB',
     '<coachee user id>', 'elimination'),
  ('41900000-0000-0000-0000-0000000000a3', '41900000-0000-0000-0000-000000000002', '300', 'Jim H',
     NULL, NULL),
  ('41900000-0000-0000-0000-0000000000a4', '41900000-0000-0000-0000-000000000002', '400', NULL,
     NULL, NULL);

INSERT INTO transcript_segments (id, transcription_id, participant_id, speaker_label, text, start_ms, end_ms) VALUES
  (gen_random_uuid(), '41900000-0000-0000-0000-000000000002', '41900000-0000-0000-0000-0000000000a1', 'J. Hodapp', 'Good morning.', 0, 1500),
  (gen_random_uuid(), '41900000-0000-0000-0000-000000000002', '41900000-0000-0000-0000-0000000000a2', 'CB', 'Morning.', 4000, 5000),
  (gen_random_uuid(), '41900000-0000-0000-0000-000000000002', '41900000-0000-0000-0000-0000000000a3', 'Jim H', 'Hi both.', 9000, 10000),
  (gen_random_uuid(), '41900000-0000-0000-0000-000000000002', '41900000-0000-0000-0000-0000000000a4', '400', 'Can you hear me?', 20000, 21000),
  (gen_random_uuid(), '41900000-0000-0000-0000-000000000002', '41900000-0000-0000-0000-0000000000a2', 'CB', 'Let us start.', 59000, 61000);
```

The fixture covers every labeling rule:

- `J. Hodapp` and `CB` are attributed, so they display as the seeded profile names `Jim H` and
  `cbourg2` (the coachee's `display_name`), not as typed.
- The unattributed `Jim H` typed the coach's exact profile name. The old name matcher would have
  called it the coach; the new code keeps it unattributed and shows it as `Jim H (2)`.
- Participant `400` has no name (a dial-in), so it shows as `Guest 1`.

```sh
login coach james.hodapp@gmail.com password
login coachee calebbourg2@gmail.com password
SESSION=<session id>
TRANSCRIPTION=41900000-0000-0000-0000-000000000002
```

### B1. Speakers come from stored attribution

```sh
meta coach
```

**Pass:** `speakers` is `Jim H` → `coach`, `cbourg2` → `coachee`, `Jim H (2)` → `null`,
`Guest 1` → `null`, in that order. Neither `J. Hodapp` nor `CB` appears.

### B2. Filter follows attribution, not names

```sh
txt coach "?speaker=coach"
```

**Pass:** 200, header `Speakers: Jim H`, one line `[0:00] Jim H: Good morning.`. `Hi both.`
(spoken by the unattributed participant who typed `Jim H`) is absent.

### B3. Both roles

```sh
txt coachee "?speaker=coach&speaker=coachee"
```

**Pass:** 200, header `Speakers: Jim H, cbourg2`, three lines; `Hi both.` and
`Can you hear me?` absent.

### B3b. Unfiltered file labels everyone

```sh
txt coach ""
```

**Pass:** 200, header `Speakers: Jim H, cbourg2, Jim H (2), Guest 1`, the 9s line reads
`[0:09] Jim H (2): Hi both.`, and the 20s line reads `[0:20] Guest 1: Can you hear me?`.

### B3c. Segments endpoint uses the same labels

```sh
segs coach
```

**Pass:** five segments whose values, in time order, are:

| `speaker_label` | `speaker_user_id` | `speaker_role` |
|---|---|---|
| `Jim H` | coach's user id | `coach` |
| `cbourg2` | coachee's user id | `coachee` |
| `Jim H (2)` | `null` | `null` |
| `Guest 1` | `null` | `null` |
| `cbourg2` | coachee's user id | `coachee` |

Every label appears in B1's `speakers` with the same role, and no `J. Hodapp`, `CB`, or `400`
appears.

### B4. Unresolved role is 422

```sql
UPDATE refactor_platform.transcript_participants
SET user_id = NULL, match_source = NULL
WHERE id = '41900000-0000-0000-0000-0000000000a2';
```

```sh
txt coach "?speaker=coachee"
```

**Pass:** 422 `speaker_not_identified`. `meta coach` now shows `Jim H` → `coach`, `CB` →
`null` (now unattributed, so shown as typed), `Jim H (2)` → `null`, `Guest 1` → `null`.
Restore with
`UPDATE ... SET user_id = '<coachee user id>', match_source = 'elimination' WHERE id = '...a2';`.

### B5. The database refuses half an attribution

```sql
UPDATE refactor_platform.transcript_participants
SET match_source = NULL
WHERE id = '41900000-0000-0000-0000-0000000000a1';
```

**Pass:** the update fails on the check constraint (a user without a source, or a source
without a user, is never stored).

### B6. Role is derived at read time

Swap the relationship's coach and coachee (SQL on `coaching_relationships`), rerun B1, then
swap back.

**Pass:** `Jim H` now shows `coachee` and `cbourg2` shows `coach`, in both `meta coach` and
`segs coach` (each segment's `speaker_role` flips; `speaker_user_id` does not change). Attribution stores users, not roles, so the role
follows whoever holds it now.

### B7. Session-level read

```sh
latest coach
```

**Pass:** if this fixture is the session's latest transcription, `speakers` equals B1's list.
Otherwise the field reflects the latest transcription's own participants.

### B8. Cleanup

```sql
DELETE FROM refactor_platform.transcriptions WHERE id = '41900000-0000-0000-0000-000000000002';
DELETE FROM refactor_platform.meeting_recordings WHERE id = '41900000-0000-0000-0000-000000000001';
```

**Pass:** participants and segments for the transcription are gone (cascade).

## Part C: backfill (operator)

Follow `transcript_speaker_attribution_backfill.md` (run from a local dev machine: read-only dry run
against production, apply on a fork, then production).

**Pass:**
- the dry run writes nothing and its CSV accounts for every completed transcription without
  participants;
- on the fork and on production, the segment fingerprint is unchanged after applying;
- a second dry run after applying reports nothing left to attribute;
- a backfilled session shows profile names and a working coach-only download.

## Results

| Case | Result | Date | Notes |
|---|---|---|---|
| A1 | | | |
| A2 | | | |
| A3 | | | |
| A4 | | | |
| A5 | | | |
| A6 | | | |
| A7 | | | |
| A8 | | | |
| A9 | | | |
| A10 | | | |
| A11 | | | |
| A12 | | | |
| A13 | PASS (API only) | 2026-10-06 | non-participant `admin@refactorcoach.com` got 403 from `meta`, `latest`, `segs` on a fixture transcript |
| A14 | | | |
| A15 | | | |
| B1 | PASS | 2026-10-06 | PR #434 preview |
| B2 | PASS | 2026-10-06 | PR #434 preview |
| B3 | PASS | 2026-10-06 | PR #434 preview |
| B3b | PASS | 2026-10-06 | PR #434 preview |
| B3c | PASS | 2026-10-06 | PR #434 preview |
| B4 | PASS | 2026-10-06 | PR #434 preview |
| B5 | PASS | 2026-10-06 | PR #434 preview |
| B6 | PASS | 2026-10-06 | PR #434 preview |
| B7 | PASS | 2026-10-06 | PR #434 preview |
| B8 | PASS | 2026-10-06 | PR #434 preview |
| C | | | |
