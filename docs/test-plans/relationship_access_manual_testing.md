# Test Plan: Relationship Access Matrix

Records who may do what through a coaching relationship today: every route, job and email
that reads or writes relationship-scoped data, per caller, with where the rule is enforced.
It is the regression reference for rs#399: coach and coachee rows must answer the same before
and after that work. Plan: [`colleague-relationships.md`](../implementation-plans/colleague-relationships.md).

Rows record what the code does today, plus the rules tightened by the relationship
authorization security PR (`fix/relationship-scoped-authorization`), which lands before the
rs#399 baseline is frozen. Rows it changes or adds are marked `[tightened]`; their
`Enforced at` cells point at the new checks. Row ids are stable; never renumber them.
Phase 4 adds colleague rows; there are none yet.

Rows marked `[membership]` change in phase M (org membership consistency), which also lands
before the baseline. Their expected results already show the phase M rule:

- A user's own lists (`/users/{self}/coaching-relationships`, `/coaching_sessions` and its
  `/counts` and `/nearest`, `/actions`) leave out relationships in orgs the user no longer
  belongs to, unless they are a SuperAdmin, matching the 403 they get opening one.
- Session and series emails, and action assignment emails, go only to participants who are
  still members of the relationship's org. Cancellations are the exception: they go to both
  participants, so a removed member's calendar is cleaned up.
- Membership is checked once per request, before any write, and that result decides who is
  emailed. A participant removed while a request is in flight can still get that one email.
- Creating, rescheduling or resending a session or series is refused (409,
  `details.reason` `coachee_not_a_member`) when the coachee is no longer a member; assigning an
  action to someone who is no longer a member is refused (`assignee_not_a_member`). Cancelling
  and deleting stay allowed (SES-33 to SES-36, SER-17 to SER-21, ACT-37, ACT-38, TRN-28).
- Deleting an account with coaching history is refused (409 `user_has_coaching_history`,
  ORG-08); removing the person from the organization is the way out.

Two behaviors are confirmed as intended and recorded as they are: a coachee's Actions page
(batch and per-relationship lists) shows only their own and unassigned actions, while every
other surface shows the session's actions in full; and org admins and SuperAdmins who are
not participants can list an org's relationships but get 403 reading one or its content.

Rules the security PR adds:

- Notes, agreements and actions, read or written by id or created for a session: the caller
  must be a participant of that session's relationship and still a member of its
  organization (`grants_access_to`). Unknown id: 404. Otherwise: 403.
- Action assignees must be the relationship's coach or coachee. Anyone else: 422, nothing
  written, no email.
- Goals created for a relationship, and goal links on a session: same participant rule. A
  goal can only be linked to a session in its own relationship, including the session a new
  goal is created in.
- The batch session-goals read authorizes every session id, not only the first.
- `POST /coaching_sessions`: only the relationship's coach, still a member of its
  organization. Series creation also requires that membership.
- `GET /users/{id}/actions` by a coach of that user: only actions from relationships where
  the caller coaches that user.

> [!WARNING]
> Several rows mutate or delete data, and the `MAIL` rows send real mail when Resend is
> configured. Use a local stack with disposable data.

## 1. Prerequisites

- Backend running locally at `http://localhost:4000` with a seeded database you can edit.
- Two organizations and eight accounts (section 2). Create them with the admin screens or
  the API rows `REL-15`, `ORG-01` and `ORG-03`.
- For `TRN` rows: a completed transcription on S1 (a recorded meeting, or a row inserted by
  hand) and `ASSEMBLY_AI_API_KEY` set if you run the scan rows.
- For `MEET` rows: COACH has a Google or Zoom connection (`/oauth/{provider}/authorize`).

## 2. Fixture

Every row refers to this cast. All in organization **O1** unless stated.

| Id | Who |
|---|---|
| `COACH` | Coach of `COACHEE` in `R1`; also coach of `REMOVED` in `R3` |
| `COACHEE` | Coachee in `R1` and in `R2` |
| `OTHER_COACH` | Coach of `COACHEE` in `R2` |
| `MEMBER` | Member (role `User`) of O1, in no relationship |
| `ADMIN` | `Admin` of O1, in no relationship |
| `OUTSIDER` | Member of **O2** only |
| `SUPER` | Global `SuperAdmin` (no organization role), in no relationship |
| `REMOVED` | Was coachee in `R3`, then removed from O1 (`ORG-04`); holds no O1 role. `R3` still exists |

Rows the scenarios need. Put the word `kumquat` in every body and session title so search rows find them.

| Id | Row |
|---|---|
| `R1` | Relationship COACH to COACHEE, O1 |
| `R2` | Relationship OTHER_COACH to COACHEE, O1 |
| `R3` | Relationship COACH to REMOVED, O1 |
| `S1`, `S2`, `S3` | One future session in R1, R2, R3 respectively |
| `SR1` | Series in R1 created by COACH (its sessions are `SR1-*`) |
| `SR3` | Series in R3 created by COACH before REMOVED is removed (its sessions are `SR3-*`); after phase M a series can't be created for a removed coachee |
| `A1` | Action in S1 assigned to COACH |
| `A2` | Action in S1 assigned to COACHEE |
| `A3` | Action in S1, unassigned |
| `A4` | Action in S2 assigned to OTHER_COACH |
| `A5` | Action in S2 assigned to COACHEE |
| `A6` | Action in S3 assigned to REMOVED |
| `G1` | Goal in R1 created by COACH, linked to S1 |
| `G2` | Goal in R2 created by OTHER_COACH, linked to S2 |
| `N1` | Note in S1 |
| `AG1` | Agreement in S1 |
| `T1` | Topic in S1 authored by COACH |
| `T2` | Topic in S1 authored by COACHEE |
| `IMG1` | Note image in S1 |
| `TR1` | Completed transcription of S1 |
| `SG1` | Suggested action in S1 assigned to COACH only |
| `SG2` | Suggested action in S1 assigned to COACHEE |
| `SG3` | Suggested action in S3 assigned to REMOVED (needs a completed transcription of S3) |

## 3. Setup and curl helpers

Login is `POST /login` with form fields `email` and `password`
(`web/src/router.rs:738`, `web/src/controller/user_session_controller.rs:30`,
`Credentials` at `entity_api/src/user.rs:421`). It sets the session cookie `id`. Every
handler requires the `x-version: 1.0.0` header (missing: 400). Without a session cookie every
route below except the webhook returns 401 (`web/src/middleware/auth.rs:13`).

```sh
BASE=http://localhost:4000
VER='x-version: 1.0.0'

# $1 = jar name, $2 = email, $3 = password
login() { curl -s -c "/tmp/$1.jar" -X POST "$BASE/login" \
  -H 'content-type: application/x-www-form-urlencoded' \
  --data-urlencode "email=$2" --data-urlencode "password=$3" \
  -o /dev/null -w '%{http_code}\n'; }

# $1 = jar, $2 = method, $3 = path with query, $4 = optional JSON body.
# Prints the transport status, then the body.
call() {
  if [ -n "$4" ]; then
    curl -s -b "/tmp/$1.jar" -H "$VER" -H 'content-type: application/json' \
      -X "$2" "$BASE$3" -d "$4" -w '\n%{http_code}\n'
  else
    curl -s -b "/tmp/$1.jar" -H "$VER" -X "$2" "$BASE$3" -w '\n%{http_code}\n'
  fi
}

for p in coach coachee other_coach member admin outsider super removed; do
  login "$p" "$p@access.test" password
done

# Then export the fixture ids, e.g.
# O1=... O2=... R1=... R2=... R3=... S1=... S2=... S3=... SR1=... SR3=...
# A1=... A2=... A3=... A4=... A5=... A6=... G1=... G2=... N1=... AG1=...
# T1=... T2=... IMG1=... TR1=... SG1=... SG2=...
# COACH_ID=... COACHEE_ID=... MEMBER_ID=... REMOVED_ID=...

call coachee GET "/organizations/$O1/coaching_relationships/$R1/actions"
```

> [!NOTE]
> Success bodies use the `ApiResponse` envelope: the transport status is 200 and the
> envelope `status_code` may say 201 or 204. `Expected` gives the transport status, with the
> envelope code in parentheses when it differs, e.g. `200 (201)`. Errors use the transport
> status directly.

How to read the tables:

- `Expected` for reads names which fixture ids come back, never a full body.
- `[rows]` in Notes marks a row whose answer depends on which rows a query selects
  (visibility filters, search, recipient lists). Phase 0b covers those with SQLite tests.
- "Non-participants" means `MEMBER`, `OTHER_COACH` (for R1 data), `ADMIN`, `SUPER` and
  `OUTSIDER`, grouped only where the code takes the identical path for all of them.
- `grants_access_to` (`entity/src/coaching_relationships.rs:87`) is the common participant
  rule: coach or coachee of the relationship AND (global SuperAdmin OR any role in the
  relationship's organization). A non-participant SuperAdmin does not pass it.

## 4. Matrix

### 4.1 Relationships (`REL`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| REL-01 | GET /organizations/O1/coaching_relationships | COACH | 200, R1 R3 | `web/src/extractors/organization_member_access.rs:85`; `domain/src/coaching_relationship.rs:63` | [rows] participant filter |
| REL-02 | GET /organizations/O1/coaching_relationships | COACHEE | 200, R1 R2 | same as REL-01 | [rows] |
| REL-03 | GET /organizations/O1/coaching_relationships | OTHER_COACH | 200, R2 | same as REL-01 | [rows] |
| REL-04 | GET /organizations/O1/coaching_relationships | MEMBER | 200, empty list | same as REL-01 | [rows] |
| REL-05 | GET /organizations/O1/coaching_relationships | ADMIN | 200, R1 R2 R3 | `domain/src/coaching_relationship.rs:49`, `:58`; `entity_api/src/user.rs:236` | [rows] admin sees every relationship in the org |
| REL-06 | GET /organizations/O1/coaching_relationships | SUPER | 200, R1 R2 R3 | `web/src/extractors/organization_member_access.rs:81`; `domain/src/coaching_relationship.rs:58` | [rows] |
| REL-07 | GET /organizations/O1/coaching_relationships | OUTSIDER, REMOVED | 403 | `web/src/extractors/organization_member_access.rs:103` | Not a member of O1 |
| REL-08 | GET /organizations/O1/coaching_relationships/R1 | COACH, COACHEE | 200, R1 with names | `web/src/extractors/coaching_relationship_access.rs:84` | |
| REL-09 | GET /organizations/O1/coaching_relationships/R1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/extractors/coaching_relationship_access.rs:84` | ADMIN and SUPER list R1 (REL-05, REL-06) but cannot read it |
| REL-10 | GET /organizations/O2/coaching_relationships/R1 | COACH | 404 | `web/src/extractors/coaching_relationship_access.rs:78` | Path org must match the relationship's |
| REL-11 | GET /organizations/O1/coaching_relationships/R3 | REMOVED | 403 | `web/src/extractors/coaching_relationship_access.rs:84` | Participant, no longer a member |
| REL-12 | GET /organizations/O1/coaching_relationships/R3 | COACH | 200, R3 | `web/src/extractors/coaching_relationship_access.rs:84` | Coach keeps access after the coachee is removed. [membership] `coach_is_member` true, `coachee_is_member` false |
| REL-13 | GET /users/{COACHEE_ID}/coaching-relationships | COACHEE | 200, R1 R2 | `web/src/protect/users/mod.rs:26`; `entity_api/src/coaching_relationship.rs:543` | [rows] self only. `role=coach`: empty; `role=coachee`: R1 R2 |
| REL-14 | GET /users/{COACHEE_ID}/coaching-relationships | COACH, MEMBER, ADMIN, SUPER | 403 | `web/src/protect/users/mod.rs:26` | Self only, no admin bypass |
| REL-15 | POST /organizations/O1/coaching_relationships `{coach_id: COACH, coachee_id: MEMBER}` | ADMIN, SUPER | 200 (201), new relationship | `web/src/extractors/organization_admin_access.rs:42`; `entity_api/src/coaching_relationship.rs:70` | Same pair again returns the existing row. Both people must be O1 members |
| REL-16 | POST /organizations/O1/coaching_relationships | COACH, COACHEE, MEMBER, OUTSIDER | 403 | `web/src/extractors/organization_admin_access.rs:48` | |
| REL-17 | GET /users/{REMOVED_ID}/coaching-relationships | REMOVED | 200, empty list | `web/src/protect/users/mod.rs:26`; `entity_api/src/coaching_relationship.rs:543` | [membership] self list leaves out R3; was 200, R3 |
| REL-18 | GET /organizations/O1/coaching_relationships | ADMIN | 200; R3 has `coach_is_member` true and `coachee_is_member` false, R1 and R2 have both true | `entity_api/src/coaching_relationship.rs:236` | [membership] Both fields are on every relationship object: org list, single read, `GET /users/{id}/coaching-relationships`, the create response (both true), and `include=relationship` on session reads |

### 4.2 Sessions (`SES`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| SES-01 | GET /coaching_sessions?coaching_relationship_id=R1 | COACH, COACHEE | 200, S1 and SR1-* | `web/src/protect/coaching_sessions.rs:33` | [rows] filtered by relationship id |
| SES-02 | GET /coaching_sessions?coaching_relationship_id=R1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/protect/coaching_sessions.rs:33` | Unknown relationship id: 404 (`:42`). Missing param: 400 |
| SES-03 | GET /coaching_sessions?coaching_relationship_id=R3 | REMOVED | 403 | `web/src/protect/coaching_sessions.rs:33` | |
| SES-04 | GET /coaching_sessions/S1 | COACH, COACHEE | 200, S1 plus `organization_id` O1 | `web/src/extractors/coaching_session_access.rs:108` | First read hydrates the session, see MEET-01 |
| SES-05 | GET /coaching_sessions/S1 | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | Unknown session: 404 (`:105`) |
| SES-06 | GET /coaching_sessions/S3 | REMOVED | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| SES-07 | POST /coaching_sessions/S1/view | COACH, COACHEE | 200, prior marker | `web/src/extractors/coaching_session_access.rs:108` | Per-caller marker |
| SES-08 | POST /coaching_sessions/S1/view | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| SES-09 | POST /coaching_sessions `{coaching_relationship_id: R1}` | COACH | 200 (201), new session in R1 | `web/src/controller/coaching_session_controller.rs:185`; `web/src/extractors/coaching_relationship_access.rs:123` | [tightened] Coach only, still a member. Sends MAIL-01 |
| SES-10 | POST /coaching_sessions `{coaching_relationship_id: R1}` | COACHEE | 403 | `web/src/controller/coaching_session_controller.rs:185`; `web/src/extractors/coaching_relationship_access.rs:123` | [tightened] Coachee cannot create; colleagues will (has_authority, rs#399 phase 3) |
| SES-11 | POST /coaching_sessions `{coaching_relationship_id: R1}` | MEMBER, OTHER_COACH | 403 | `web/src/controller/coaching_session_controller.rs:185`; `web/src/extractors/coaching_relationship_access.rs:123` | [tightened] Unknown relationship: 404 |
| SES-12 | POST /coaching_sessions `{coaching_relationship_id: R1}` | ADMIN, SUPER | 403 | `web/src/controller/coaching_session_controller.rs:185`; `web/src/extractors/coaching_relationship_access.rs:123` | [tightened] |
| SES-13 | POST /coaching_sessions `{coaching_relationship_id: R1}` | OUTSIDER | 403 | `web/src/controller/coaching_session_controller.rs:185`; `web/src/extractors/coaching_relationship_access.rs:123` | [tightened] |
| SES-14 | POST /coaching_sessions `{coaching_relationship_id: R3}` | REMOVED | 403 | `web/src/controller/coaching_session_controller.rs:185`; `web/src/extractors/coaching_relationship_access.rs:123` | [tightened] Not the coach, and no longer a member |
| SES-15 | PUT /coaching_sessions/S1 | COACH | 200 (204) | `web/src/protect/coaching_sessions.rs:129` | Coach only. A date change queues JOB-02 |
| SES-16 | PUT /coaching_sessions/S1 | COACHEE | 403 | `web/src/protect/coaching_sessions.rs:129` | |
| SES-17 | PUT /coaching_sessions/S1 | non-participants | 403 | `web/src/protect/coaching_sessions.rs:129` | Unknown session: 404 |
| SES-18 | PUT /coaching_sessions/S3 | COACH | 200 (204) | `web/src/protect/coaching_sessions.rs:129` | Coach still a member after the coachee left. [membership] Only when `date` and `duration_minutes` are unchanged, see SES-34 |
| SES-19 | PATCH /coaching_sessions/S1/title | COACH, COACHEE | 200, S1 with new title | `web/src/extractors/coaching_session_access.rs:108` | Either participant |
| SES-20 | PATCH /coaching_sessions/S1/title | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| SES-21 | DELETE /coaching_sessions/S1 | COACH | 200 (204) | `web/src/protect/coaching_sessions.rs:129` | Sends MAIL-03 |
| SES-22 | DELETE /coaching_sessions/S1 | COACHEE | 403 | `web/src/protect/coaching_sessions.rs:129` | |
| SES-23 | DELETE /coaching_sessions/S1 | non-participants | 403 | `web/src/protect/coaching_sessions.rs:129` | |
| SES-24 | POST /coaching_sessions/S1/resend_invite | COACH | 204 | `web/src/protect/coaching_sessions.rs:129` | 409 when past or a series member. Sends MAIL-04 |
| SES-25 | POST /coaching_sessions/S1/resend_invite | COACHEE | 403 | `web/src/protect/coaching_sessions.rs:129` | |
| SES-26 | POST /coaching_sessions/S1/resend_invite | non-participants | 403 | `web/src/protect/coaching_sessions.rs:129` | |
| SES-27 | GET /users/{COACH_ID}/coaching_sessions | COACH | 200, S1 S3 SR1-* | `web/src/protect/users/coaching_sessions.rs:20`; `entity_api/src/coaching_session.rs:1176` | [rows] self only. `include=relationship` adds `coach`/`coachee` |
| SES-28 | GET /users/{COACHEE_ID}/coaching_sessions | COACHEE | 200, S1 S2 SR1-* | same as SES-27 | [rows] |
| SES-29 | GET /users/{COACHEE_ID}/coaching_sessions | COACH, MEMBER, ADMIN, SUPER | 403 | `web/src/protect/users/coaching_sessions.rs:20` | Same gate for `/counts` and `/nearest` |
| SES-30 | GET /users/{REMOVED_ID}/coaching_sessions | REMOVED | 200, empty list | `web/src/protect/users/coaching_sessions.rs:20` | [membership] self list leaves out S3; was 200, S3 |
| SES-31 | GET /users/{COACHEE_ID}/coaching_sessions/counts | COACHEE | 200, counts over S1 S2 SR1-* | `web/src/protect/users/coaching_sessions.rs:20`; `entity_api/src/coaching_session.rs:877` | [rows] |
| SES-32 | GET /users/{COACHEE_ID}/coaching_sessions/nearest?coaching_relationship_ids=R1,R2 | COACHEE | 200, nearest of R1 and of R2 | `web/src/protect/users/coaching_sessions.rs:20`; `entity_api/src/coaching_session.rs:1206` | [rows] a relationship the user is not in contributes nothing |
| SES-33 | POST /coaching_sessions `{coaching_relationship_id: R3}` | COACH | 409 `coachee_not_a_member`, nothing written, no email | `domain/src/coaching_session.rs:102` | [membership] `details: {reason, organization_id, user_id}` |
| SES-34 | PUT /coaching_sessions/S3 changing `date` or `duration_minutes` | COACH | 409 `coachee_not_a_member`, S3 unchanged | `domain/src/coaching_session.rs:390` | [membership] Other edits succeed (SES-18) |
| SES-35 | POST /coaching_sessions/S3/resend_invite | COACH | 409 `coachee_not_a_member`, no email | `domain/src/coaching_session.rs:565` | [membership] |
| SES-36 | DELETE /coaching_sessions/S3 | COACH | 200 (204) | `web/src/protect/coaching_sessions.rs:129` | [membership] Deleting stays allowed. Sends MAIL-03 to both COACH and REMOVED. Run after SES-34 and SES-35 |

### 4.3 Series (`SER`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| SER-01 | POST /coaching_session_series `{coaching_relationship_id: R1}` | COACH | 200 (201), series and its sessions | `domain/src/coaching_session_series.rs:92` | Sends MAIL-05 |
| SER-02 | POST /coaching_session_series `{coaching_relationship_id: R1}` | COACHEE | 403 | `domain/src/coaching_session_series.rs:92` | |
| SER-03 | POST /coaching_session_series `{coaching_relationship_id: R1}` | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `domain/src/coaching_session_series.rs:92` | Unknown relationship: 404. [tightened] Coach check now also requires membership, see SER-16 |
| SER-04 | GET /coaching_session_series?coaching_relationship_id=R1 | COACH, COACHEE | 200, SR1 | `web/src/extractors/coaching_session_series_access.rs:88` | [rows] |
| SER-05 | GET /coaching_session_series?coaching_relationship_id=R1 | non-participants | 403 | `web/src/extractors/coaching_session_series_access.rs:88` | |
| SER-06 | GET /coaching_session_series/SR1 | COACH, COACHEE | 200, SR1 and its sessions | `web/src/extractors/coaching_session_series_access.rs:36` | |
| SER-07 | GET /coaching_session_series/SR1 | non-participants | 403 | `web/src/extractors/coaching_session_series_access.rs:36` | Unknown series: 404 |
| SER-08 | PUT /coaching_session_series/SR1 | COACH | 200 | `web/src/extractors/coaching_session_series_access.rs:110` | Sends MAIL-06 |
| SER-09 | PUT /coaching_session_series/SR1 | COACHEE, non-participants | 403 | `web/src/extractors/coaching_session_series_access.rs:110` | |
| SER-10 | POST /coaching_session_series/SR1/reschedule | COACH | 200 | `web/src/extractors/coaching_session_series_access.rs:110` | Sends MAIL-07 |
| SER-11 | POST /coaching_session_series/SR1/reschedule | COACHEE, non-participants | 403 | `web/src/extractors/coaching_session_series_access.rs:110` | |
| SER-12 | DELETE /coaching_session_series/SR1 | COACH | 204 | `web/src/extractors/coaching_session_series_access.rs:110` | Sends MAIL-08 |
| SER-13 | DELETE /coaching_session_series/SR1 | COACHEE, non-participants | 403 | `web/src/extractors/coaching_session_series_access.rs:110` | |
| SER-14 | POST /coaching_session_series/SR1/resend_invite | COACH | 204 | `web/src/extractors/coaching_session_series_access.rs:110` | Sends MAIL-09 |
| SER-15 | POST /coaching_session_series/SR1/resend_invite | COACHEE, non-participants | 403 | `web/src/extractors/coaching_session_series_access.rs:110` | |
| SER-16 | POST /coaching_session_series `{coaching_relationship_id: R1}` | COACH after removal from O1 | 403 | `web/src/controller/coaching_session_series_controller.rs:60`; `web/src/extractors/coaching_relationship_access.rs:123` | [tightened] Run last, or on a disposable relationship: remove the coach from O1 first |
| SER-17 | POST /coaching_session_series `{coaching_relationship_id: R3}` | COACH | 409 `coachee_not_a_member`, nothing written, no email | `domain/src/coaching_session_series.rs:103` | [membership] |
| SER-18 | PUT /coaching_session_series/SR3 | COACH | 409 `coachee_not_a_member`, SR3 unchanged | `domain/src/coaching_session_series.rs:257` | [membership] |
| SER-19 | POST /coaching_session_series/SR3/reschedule | COACH | 409 `coachee_not_a_member`, SR3 unchanged | `domain/src/coaching_session_series.rs:645` | [membership] |
| SER-20 | POST /coaching_session_series/SR3/resend_invite | COACH | 409 `coachee_not_a_member`, no email | `domain/src/coaching_session_series.rs:913` | [membership] |
| SER-21 | DELETE /coaching_session_series/SR3 | COACH | 204 | `web/src/extractors/coaching_session_series_access.rs:110` | [membership] Cancelling stays allowed. Sends MAIL-08 to both COACH and REMOVED. Run after SER-18 to SER-20 |

### 4.4 Notes (`NOTE`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| NOTE-01 | GET /notes?coaching_session_id=S1 | COACH, COACHEE | 200, N1 | `web/src/protect/notes.rs:34` | [rows] |
| NOTE-02 | GET /notes?coaching_session_id=S1 | non-participants | 403 | `web/src/protect/notes.rs:34` | |
| NOTE-03 | GET /notes?coaching_session_id=S3 | REMOVED | 403 | `web/src/protect/notes.rs:34` | |
| NOTE-04 | GET /notes/N1 | COACH, COACHEE | 200, N1 | `web/src/extractors/note_access.rs:49` | [tightened] |
| NOTE-05 | POST /notes `{coaching_session_id: S1}` | COACH, COACHEE | 200 (201), new note | `web/src/controller/note_controller.rs:46` | [tightened] |
| NOTE-06 | PUT /notes/N1 | COACH, COACHEE | 200, N1 updated | `web/src/extractors/note_access.rs:49` | [tightened] |
| NOTE-07 | GET /notes/N1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/extractors/note_access.rs:49` | [tightened] Unknown note: 404 |
| NOTE-08 | POST /notes `{coaching_session_id: S1}` | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, nothing written | `web/src/controller/note_controller.rs:46` | [tightened] Unknown session: 404 |
| NOTE-09 | PUT /notes/N1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, N1 unchanged | `web/src/extractors/note_access.rs:49` | [tightened] |
| NOTE-10 | GET, PUT a note in S3; POST `{coaching_session_id: S3}` | REMOVED | 403 | `web/src/extractors/note_access.rs:49`; `web/src/controller/note_controller.rs:46` | [tightened] |

### 4.5 Agreements (`AGR`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| AGR-01 | GET /agreements?coaching_session_id=S1 | COACH, COACHEE | 200, AG1 | `web/src/protect/agreements.rs:29` | [rows] |
| AGR-02 | GET /agreements?coaching_session_id=S1 | non-participants | 403 | `web/src/protect/agreements.rs:29` | |
| AGR-03 | GET /agreements?coaching_session_id=S3 | REMOVED | 403 | `web/src/protect/agreements.rs:29` | |
| AGR-04 | GET /agreements/AG1 | COACH, COACHEE | 200, AG1 | `web/src/extractors/agreement_access.rs:48` | [tightened] |
| AGR-05 | POST /agreements `{coaching_session_id: S1}` | COACH, COACHEE | 200 (201), new agreement | `web/src/controller/agreement_controller.rs:49` | [tightened] |
| AGR-06 | PUT /agreements/AG1 | COACH, COACHEE | 200, AG1 updated | `web/src/extractors/agreement_access.rs:48` | [tightened] |
| AGR-07 | DELETE /agreements/AG1 | COACH, COACHEE | 200, `{id}` | `web/src/extractors/agreement_access.rs:48` | [tightened] Run last in this table |
| AGR-08 | GET /agreements/AG1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/extractors/agreement_access.rs:48` | [tightened] Unknown agreement: 404 |
| AGR-09 | POST /agreements `{coaching_session_id: S1}` | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, nothing written | `web/src/controller/agreement_controller.rs:49` | [tightened] Unknown session: 404 |
| AGR-10 | PUT, DELETE /agreements/AG1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, AG1 unchanged | `web/src/extractors/agreement_access.rs:48` | [tightened] |
| AGR-11 | GET, PUT, DELETE an agreement in S3; POST `{coaching_session_id: S3}` | REMOVED | 403 | `web/src/extractors/agreement_access.rs:48`; `web/src/controller/agreement_controller.rs:49` | [tightened] |

### 4.6 Actions (`ACT`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| ACT-01 | GET /organizations/O1/coaching_relationships/actions | COACH | 200, `coachee_actions`: R1 [A1 A2 A3], R3 [A6] | `web/src/extractors/organization_member_access.rs:85`; `entity_api/src/action.rs:598` | [rows] keyed by relationship id |
| ACT-02 | GET /organizations/O1/coaching_relationships/actions | COACHEE | 200, R1 [A2 A3], R2 [A5]; not A1, A4 | `entity_api/src/action.rs:482`, `:464` | [rows] coachee side sees own or unassigned |
| ACT-03 | GET /organizations/O1/coaching_relationships/actions | OTHER_COACH | 200, R2 [A4 A5] | `entity_api/src/action.rs:598` | [rows] |
| ACT-04 | GET /organizations/O1/coaching_relationships/actions | MEMBER, ADMIN, SUPER | 200, empty object | `web/src/controller/organization/coaching_relationship/actions_controller.rs:158` | [rows] only the caller's own relationships |
| ACT-05 | GET /organizations/O1/coaching_relationships/actions | OUTSIDER, REMOVED | 403 | `web/src/extractors/organization_member_access.rs:103` | |
| ACT-06 | GET /organizations/O1/coaching_relationships/actions?assignee=coach | COACHEE | 403 | `web/src/controller/organization/coaching_relationship/actions_controller.rs:169`, `:45` | Only when the caller coaches no relationship |
| ACT-07 | GET /organizations/O1/coaching_relationships/R1/actions | COACH | 200, A1 A2 A3 | `web/src/extractors/coaching_relationship_access.rs:84`; `web/src/controller/organization/coaching_relationship/actions_controller.rs:98` | [rows] |
| ACT-08 | GET /organizations/O1/coaching_relationships/R1/actions | COACHEE | 200, A2 A3; not A1 | `web/src/controller/organization/coaching_relationship/actions_controller.rs:98`; `entity_api/src/action.rs:563` | [rows] |
| ACT-09 | GET /organizations/O1/coaching_relationships/R1/actions?assignee=coach | COACHEE | 403 | `web/src/controller/organization/coaching_relationship/actions_controller.rs:100` | `assignee=coachee` or own id: 200, A2 |
| ACT-10 | GET /organizations/O1/coaching_relationships/R1/actions | MEMBER, OTHER_COACH, ADMIN, SUPER | 403 | `web/src/extractors/coaching_relationship_access.rs:84` | |
| ACT-11 | GET /organizations/O1/coaching_relationships/R1/actions | OUTSIDER | 403 | `web/src/extractors/organization_member_access.rs:103` | Member check runs first |
| ACT-12 | GET /organizations/O1/coaching_relationships/R3/actions | REMOVED | 403 | `web/src/extractors/organization_member_access.rs:103` | |
| ACT-13 | GET /organizations/O1/coaching_relationships/R3/actions | COACH | 200, A6 | `web/src/extractors/coaching_relationship_access.rs:84` | [rows] |
| ACT-14 | GET /actions?coaching_session_id=S1 | COACH | 200, A1 A2 A3 | `web/src/protect/actions.rs:34` | [rows] |
| ACT-15 | GET /actions?coaching_session_id=S1 | COACHEE | 200, A1 A2 A3 | `web/src/protect/actions.rs:34`; `web/src/controller/action_controller.rs:300` | [rows] no assignee narrowing here |
| ACT-16 | GET /actions?coaching_session_id=S1 | non-participants | 403 | `web/src/protect/actions.rs:34` | |
| ACT-17 | GET /actions?coaching_session_id=S3 | REMOVED | 403 | `web/src/protect/actions.rs:34` | |
| ACT-18 | GET /users/{COACHEE_ID}/actions (scope=sessions, default) | COACHEE | 200, A1 A2 A3 A4 A5 | `web/src/protect/users/actions.rs:24`; `entity_api/src/action.rs:359` | [rows] self access; no assignee narrowing |
| ACT-19 | GET /users/{COACHEE_ID}/actions?scope=assigned | COACHEE | 200, A2 A5 | `web/src/protect/users/actions.rs:24`; `entity_api/src/action.rs:350` | [rows] |
| ACT-20 | GET /users/{COACH_ID}/actions (scope=sessions) | COACH | 200, A1 A2 A3 A6 | `web/src/protect/users/actions.rs:24`; `entity_api/src/action.rs:359` | [rows] `scope=assigned`: A1 |
| ACT-21 | GET /users/{REMOVED_ID}/actions (scope=sessions) | REMOVED | 200, empty list | `web/src/protect/users/actions.rs:24`; `entity_api/src/action.rs:359` | [membership] self list leaves out A6; was 200, A6 |
| ACT-22 | GET /users/{COACHEE_ID}/actions (scope=sessions) | COACH | 200, A1 A2 A3; not A4 A5 | `web/src/protect/users/actions.rs:16`; `web/src/controller/user/action_controller.rs:74`; `entity_api/src/action.rs:395`; `entity_api/src/coaching_relationship.rs:349` | [rows] [tightened] only relationships where the caller coaches this user |
| ACT-23 | GET /users/{COACHEE_ID}/actions?scope=assigned | COACH | 200, A2; not A5 | `web/src/protect/users/actions.rs:16`; `web/src/controller/user/action_controller.rs:74`; `entity_api/src/action.rs:395`; `entity_api/src/coaching_relationship.rs:349` | [rows] [tightened] |
| ACT-24 | GET /users/{COACHEE_ID}/actions (scope=sessions) | OTHER_COACH | 200, A4 A5; not A1 A2 A3 | `web/src/protect/users/actions.rs:16`; `web/src/controller/user/action_controller.rs:74`; `entity_api/src/action.rs:395`; `entity_api/src/coaching_relationship.rs:349` | [rows] [tightened] |
| ACT-25 | GET /users/{COACHEE_ID}/actions | MEMBER, ADMIN, SUPER | 403 | `web/src/protect/users/actions.rs:16`; `web/src/controller/user/action_controller.rs:74`; `entity_api/src/action.rs:395`; `entity_api/src/coaching_relationship.rs:349` | [tightened] |
| ACT-26 | GET /users/{COACH_ID}/actions | COACHEE | 403 | `web/src/protect/users/actions.rs:16`; `web/src/controller/user/action_controller.rs:74`; `entity_api/src/action.rs:395`; `entity_api/src/coaching_relationship.rs:349` | [tightened] A coachee is not the coach's coach |
| ACT-26b | GET /users/{COACHEE_ID}/actions | COACH after their O1 role is removed | 403 | `web/src/protect/users/actions.rs:31`; `entity_api/src/coaching_relationship.rs:349` | [tightened] A coach must still be a member of the relationship's org; a coach still in another org sees only that org's relationships. Restore the role afterwards |
| ACT-27 | GET /actions/A1 | COACH, COACHEE | 200, A1 | `web/src/extractors/action_access.rs:50` | [tightened] |
| ACT-28 | POST /actions `{coaching_session_id: S1, assignee_ids: [COACHEE_ID]}` | COACH, COACHEE | 200 (201), new action | `web/src/controller/action_controller.rs:106` | [tightened] Sends MAIL-11 |
| ACT-29 | PUT /actions/A1 | COACH, COACHEE | 200, A1 updated | `web/src/extractors/action_access.rs:50` | [tightened] Newly added assignees get MAIL-11 |
| ACT-30 | PUT /actions/A1/status?value=completed | COACH, COACHEE | 200, A1 | `web/src/extractors/action_access.rs:50` | [tightened] |
| ACT-31 | DELETE /actions/A1 | COACH, COACHEE | 200, `{id}` | `web/src/extractors/action_access.rs:50` | [tightened] Run last in this table |
| ACT-32 | GET /actions/A1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/extractors/action_access.rs:50` | [tightened] Unknown action: 404 |
| ACT-33 | POST /actions `{coaching_session_id: S1}` | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, nothing written, no email | `web/src/controller/action_controller.rs:106` | [tightened] Unknown session: 404 |
| ACT-34 | PUT /actions/A1, PUT /actions/A1/status, DELETE /actions/A1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, A1 unchanged | `web/src/extractors/action_access.rs:50` | [tightened] |
| ACT-35 | GET, PUT, DELETE A6; POST `{coaching_session_id: S3}` | REMOVED | 403 | `web/src/extractors/action_access.rs:50`; `web/src/controller/action_controller.rs:106` | [tightened] |
| ACT-36 | POST /actions `{coaching_session_id: S1, assignee_ids: [MEMBER_ID]}`; PUT /actions/A3 adding MEMBER_ID | COACH | 422, nothing written, no email | `web/src/controller/action_controller.rs:61` (via `validate_links`, `:110`, `:244`) | [tightened] Assignees must be the relationship's coach or coachee |
| ACT-37 | POST /actions `{coaching_session_id: S3, assignee_ids: [REMOVED_ID]}` | COACH | 409 `assignee_not_a_member`, nothing written, no email | `web/src/controller/action_controller.rs:75`; `domain/src/coaching_relationship.rs:105` | [membership] `details: {reason, organization_id, user_id}` |
| ACT-38 | PUT /actions/A6 editing the body, REMOVED still assigned | COACH | 200, A6 updated, no email | `web/src/controller/action_controller.rs:75` | [membership] Only newly added assignees are checked |

### 4.7 Goals (`GOAL`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| GOAL-01 | GET /goals?coaching_relationship_id=R1 | COACH, COACHEE | 200, G1 | `web/src/protect/goals.rs:35` | [rows] |
| GOAL-02 | GET /goals?coaching_relationship_id=R1 | non-participants | 403 | `web/src/protect/goals.rs:35` | |
| GOAL-03 | GET /goals?coaching_relationship_id=R3 | REMOVED | 403 | `web/src/protect/goals.rs:35` | |
| GOAL-04 | GET /goals/G1 | COACH, COACHEE | 200, G1 | `web/src/protect/goals.rs:73` | Same gate: PUT, DELETE, PUT `/status`, GET `/sessions`, GET `/progress` |
| GOAL-05 | GET /goals/G1 | non-participants | 403 | `web/src/protect/goals.rs:73` | Unknown goal: 404 |
| GOAL-06 | PUT /goals/G1 | COACH, COACHEE | 200, G1 updated | `web/src/protect/goals.rs:73` | |
| GOAL-07 | DELETE /goals/G1 | COACH, COACHEE | 200, `{id}` | `web/src/protect/goals.rs:73` | Either participant |
| GOAL-08 | PUT, DELETE /goals/G1 | non-participants | 403 | `web/src/protect/goals.rs:73` | |
| GOAL-09 | GET /goals/G1/sessions | COACH, COACHEE | 200, S1 | `web/src/protect/goals.rs:73` | [rows] |
| GOAL-10 | POST /goals `{coaching_relationship_id: R1}` | COACH, COACHEE | 200 (201), new goal | `web/src/controller/goal_controller.rs:81` | [tightened] Non-participants: GOAL-25 |
| GOAL-11 | GET /coaching_sessions/S1/goals | COACH, COACHEE | 200, G1 | `web/src/protect/goals.rs:114` | [rows] |
| GOAL-12 | GET /coaching_sessions/S1/goals | non-participants | 403 | `web/src/protect/goals.rs:114` | |
| GOAL-13 | POST /coaching_sessions/S1/goals `{goal_id: G1}` | COACH, COACHEE | 200 (201), link | `web/src/controller/coaching_session/goal_controller.rs:47` | [tightened] Non-participants: GOAL-26 |
| GOAL-14 | DELETE /coaching_sessions/S1/goals/G1 | COACH, COACHEE | 200, `{goal_id}` | `web/src/controller/coaching_session/goal_controller.rs:97` | [tightened] Non-participants: GOAL-26 |
| GOAL-15 | GET /coaching_sessions/goals?coaching_relationship_id=R1 | COACH, COACHEE | 200, `session_goals` S1 [G1], SR1-* [] | `web/src/protect/goals.rs:163` | [rows] |
| GOAL-16 | GET /coaching_sessions/goals?coaching_relationship_id=R1 | non-participants | 403 | `web/src/protect/goals.rs:163` | |
| GOAL-17 | GET /coaching_sessions/goals?coaching_session_ids=S1,S2 | COACH | 403 | `web/src/protect/goals.rs:141` | [tightened] Every id is authorized; S2 is in R2. `S1` alone: 200, S1 [G1] |
| GOAL-18 | GET /coaching_sessions/goals?coaching_session_ids=S1 | non-participants | 403 | `web/src/protect/goals.rs:163` | |
| GOAL-19 | GET /organizations/O1/coaching_relationships/R1/goal_progress | COACH, COACHEE | 200, progress for G1 | `web/src/extractors/coaching_relationship_access.rs:84` | [rows] `assignee=coach`/`coachee`/participant id allowed |
| GOAL-20 | GET /organizations/O1/coaching_relationships/R1/goal_progress?assignee={MEMBER_ID} | COACH | 400 | `web/src/controller/organization/coaching_relationship_controller.rs:219` | Non-participant id rejected |
| GOAL-21 | GET /organizations/O1/coaching_relationships/R1/goal_progress | non-participants | 403 | `web/src/extractors/coaching_relationship_access.rs:84` | No separate member extractor on this route |
| GOAL-22 | GET /users/{COACH_ID}/goals | COACH | 200, G1 | `web/src/protect/users/goals.rs:20` | [rows] goals the user created |
| GOAL-23 | GET /users/{COACHEE_ID}/goals | COACHEE | 200, empty list | `web/src/protect/users/goals.rs:20` | [rows] COACHEE created no goal |
| GOAL-24 | GET /users/{COACHEE_ID}/goals | COACH, MEMBER, ADMIN, SUPER | 403 | `web/src/protect/users/goals.rs:20` | Self only |
| GOAL-25 | POST /goals `{coaching_relationship_id: R1}` | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, nothing written | `web/src/controller/goal_controller.rs:81` | [tightened] Unknown relationship: 404. REMOVED on R3: 403 |
| GOAL-26 | POST /coaching_sessions/S1/goals, DELETE /coaching_sessions/S1/goals/G1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, links unchanged | `web/src/controller/coaching_session/goal_controller.rs:47`, `:97` | [tightened] REMOVED on S3: 403 |
| GOAL-27 | POST /coaching_sessions/S1/goals `{goal_id: G2}` | COACH | 422, nothing linked | `web/src/controller/coaching_session/goal_controller.rs:59` | [tightened] G2 belongs to R2; a goal links only to sessions in its own relationship |
| GOAL-28 | POST /goals `{coaching_relationship_id: R1, created_in_session_id: S2}` | COACH | 422, nothing written | `web/src/controller/goal_controller.rs:89` | [tightened] S2 is in R2; a new goal's created-in session must be in its relationship. Unknown session: 404 |

### 4.8 Topics (`TOP`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| TOP-01 | GET /coaching_sessions/S1/topics | COACH, COACHEE | 200, T1 T2 | `web/src/extractors/coaching_session_access.rs:108` | [rows] |
| TOP-02 | GET /coaching_sessions/S1/topics | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| TOP-03 | POST /coaching_sessions/S1/topics | COACH, COACHEE | 200 (201), new topic authored by caller | `web/src/extractors/coaching_session_access.rs:108` | |
| TOP-04 | POST /coaching_sessions/S1/topics | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| TOP-05 | PATCH /coaching_sessions/S1/topics/reorder | COACH, COACHEE | 200, T1 T2 reordered | `web/src/extractors/coaching_session_access.rs:108` | |
| TOP-06 | PUT /coaching_sessions/S1/topics/T2 | COACH | 200, T2 | `web/src/extractors/coaching_session_topic_access.rs:46` | Any participant edits any topic |
| TOP-07 | PUT /coaching_sessions/S1/topics/T1 | COACHEE | 200, T1 | `web/src/extractors/coaching_session_topic_access.rs:46` | |
| TOP-08 | PUT /coaching_sessions/S1/topics/T1 | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:37` | Topic not in S1: 404 |
| TOP-09 | DELETE /coaching_sessions/S1/topics/T2 | COACH | 200 | `web/src/extractors/coaching_session_topic_access.rs:90` | Coach deletes any topic |
| TOP-10 | DELETE /coaching_sessions/S1/topics/T2 | COACHEE | 200 | `web/src/extractors/coaching_session_topic_access.rs:78` | Author |
| TOP-11 | DELETE /coaching_sessions/S1/topics/T1 | COACHEE | 404 | `web/src/extractors/coaching_session_topic_access.rs:94` | Not author, not coach |
| TOP-12 | DELETE /coaching_sessions/S1/topics/T1 | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:72` | |
| TOP-13 | PATCH /coaching_sessions/S1/topics/T1/rating | COACHEE | 200, T1 | `web/src/extractors/coaching_session_topic_access.rs:177` | Coachee only |
| TOP-14 | PATCH /coaching_sessions/S1/topics/T1/rating | COACH | 403 | `web/src/extractors/coaching_session_topic_access.rs:177` | |
| TOP-15 | PATCH /coaching_sessions/S1/topics/T1/rating | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:164` | |
| TOP-16 | PATCH /coaching_sessions/S1/topics/T1/status | COACH, COACHEE | 200, T1 | `web/src/extractors/coaching_session_topic_access.rs:46` | Either participant |
| TOP-17 | PATCH /coaching_sessions/S1/topics/T1/status | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:37` | |
| TOP-18 | POST /coaching_sessions/S1/topics/T2/undo (T2 deleted by COACH) | COACH | 404 | `web/src/extractors/coaching_session_topic_access.rs:137` | Undoing a delete is author only |
| TOP-19 | POST /coaching_sessions/S1/topics/T2/undo (T2 deleted) | COACHEE | 200, T2 restored | `web/src/extractors/coaching_session_topic_access.rs:137` | |
| TOP-20 | POST /coaching_sessions/{dest}/topics/T1/undo (T1 deferred) | COACH, COACHEE | 200, T1 back in S1 | `web/src/extractors/coaching_session_topic_access.rs:134` | Defer undo open to either participant |
| TOP-21 | POST /coaching_sessions/S1/topics/T1/undo | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:120` | |

### 4.9 Transcripts, recordings and images (`TRN`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| TRN-01 | GET /coaching_sessions/S1/transcriptions | COACH, COACHEE | 200, TR1 with `speakers` | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-02 | GET /coaching_sessions/S1/transcriptions | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-03 | GET /coaching_sessions/S1/transcriptions/TR1 (`Accept: text/plain`, `speaker=coach`) | COACH, COACHEE | 200, coach lines only | `web/src/extractors/coaching_session_access.rs:108`; `domain/src/transcription.rs:579` | 422 if no speaker attributed to the role |
| TRN-04 | GET /coaching_sessions/S1/transcriptions/TR1 | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | TR1 under another session: 404 |
| TRN-05 | GET /coaching_sessions/S1/transcriptions/TR1/transcription_segments | COACH, COACHEE | 200, TR1 segments | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-06 | GET /coaching_sessions/S1/transcriptions/TR1/transcription_segments | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-07 | GET /coaching_sessions/S1/transcript_suggestions | COACH, COACHEE | 200, SG1 SG2 | `web/src/extractors/coaching_session_access.rs:108`; `domain/src/transcript_suggestion/review.rs:246` | [rows] coachee also sees coach-only SG1 |
| TRN-08 | GET /coaching_sessions/S1/transcript_suggestions | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-09 | GET, POST /coaching_sessions/S1/transcript_suggestions/scan | COACH, COACHEE | GET 200, POST 202; scan status | `web/src/extractors/coaching_session_access.rs:108` | POST: 409 if running or done |
| TRN-10 | GET, POST /coaching_sessions/S1/transcript_suggestions/scan | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-11 | POST /coaching_sessions/S1/transcript_suggestions/SG1/accept | COACH | 200 (201), new action | `domain/src/transcript_suggestion/review.rs:553`, `:201` | Coach may always act. Sends MAIL-11 |
| TRN-12 | POST /coaching_sessions/S1/transcript_suggestions/SG1/accept | COACHEE | 403 | `domain/src/transcript_suggestion/review.rs:202` | Coach-only suggestion |
| TRN-13 | POST /coaching_sessions/S1/transcript_suggestions/SG2/accept | COACHEE | 200 (201), new action | `domain/src/transcript_suggestion/review.rs:202` | Edits that add or remove the coach: 403 |
| TRN-14 | POST /coaching_sessions/S1/transcript_suggestions/SG1/accept | non-participants | 404 | `web/src/extractors/transcript_suggestion_access.rs:40` | All failures collapse to 404 |
| TRN-15 | POST .../SG1/dismiss, .../SG1/restore | COACH | 204 | `domain/src/transcript_suggestion/review.rs:425`, `:553` | |
| TRN-16 | POST .../SG1/dismiss, .../SG1/restore | COACHEE | 403 | `domain/src/transcript_suggestion/review.rs:202` | SG2: 204 |
| TRN-17 | POST .../SG1/dismiss, .../SG1/restore | non-participants | 404 | `web/src/extractors/transcript_suggestion_access.rs:40` | |
| TRN-18 | GET /coaching_sessions/S1/meeting_recording | COACH, COACHEE | 200, latest recording or null | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-19 | POST /coaching_sessions/S1/meeting_recording `{meeting_url}` | COACH, COACHEE | 200 (201), recording | `web/src/extractors/coaching_session_access.rs:108` | Either participant starts a bot; 409 if one is active |
| TRN-20 | DELETE /coaching_sessions/S1/meeting_recording | COACH, COACHEE | 200 | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-21 | GET, POST, DELETE /coaching_sessions/S1/meeting_recording | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-22 | GET, POST, DELETE /coaching_sessions/S3/meeting_recording | REMOVED | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-23 | POST /coaching_sessions/S1/images | COACH, COACHEE | 200 (201), image | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-24 | POST /coaching_sessions/S1/images | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:108` | |
| TRN-25 | GET /coaching_session_images/IMG1 | COACH, COACHEE | 302 to storage, or 200 bytes | `web/src/extractors/coaching_session_image_access.rs:54` | |
| TRN-26 | DELETE, POST restore /coaching_session_images/IMG1 | COACH, COACHEE | 200, IMG1 | `web/src/extractors/coaching_session_image_access.rs:54` | Either participant, not only the uploader |
| TRN-27 | GET, DELETE /coaching_session_images/IMG1 | non-participants | 403 | `web/src/extractors/coaching_session_image_access.rs:54` | Unknown image: 404 |
| TRN-28 | POST /coaching_sessions/S3/transcript_suggestions/SG3/accept | COACH | 409 `assignee_not_a_member`, no action written, no email | `domain/src/transcript_suggestion/review.rs:339` | [membership] |

### 4.10 Meeting links and OAuth-dependent flows (`MEET`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| MEET-01 | GET /coaching_sessions/SR1-x (not yet hydrated) | COACHEE | 200, session with provider and `meeting_url` from COACH's connection | `web/src/extractors/coaching_session_access.rs:108`; `domain/src/coaching_session.rs:259` | Hydration always uses the coach's account, whoever reads first |
| MEET-02 | POST /coaching_sessions with `provider` | caller per SES-09 to SES-14 | as SES-09 to SES-14; a link is attached only on SES-09 | `domain/src/coaching_session.rs:111` | [tightened] Link comes from the coach's account or a reused relationship link (`:564`) |
| MEET-03 | PUT /coaching_sessions/S1 `{meeting_url, provider}` | COACH | 200 (204) | `web/src/protect/coaching_sessions.rs:129` | Coach only; COACHEE 403 (SES-16) |
| MEET-04 | POST /coaching_sessions/S1/meeting_recording `{meeting_url}` | COACHEE | 200 (201) | `web/src/extractors/coaching_session_access.rs:108` | URL is caller supplied; not tied to an account |
| MEET-05 | GET /oauth/connections, GET/DELETE /oauth/connections/{provider} | any authenticated caller | 200, caller's own connections | `web/src/router.rs:902` (auth only) | Self scoped, no relationship data. Listed so MEET-01 has its prerequisite |

### 4.11 Search (`SRCH`)

All rows: `GET /search?q=kumquat&types=actions`. Rows are scoped by
`entity/src/coaching_relationships.rs:124` (`visible_to`: participant AND member of the
relationship's org) via `entity_api/src/search/visibility.rs:18`; SuperAdmins skip it.

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| SRCH-01 | GET /search?q=kumquat&types=actions | COACH | 200, A1 A2 A3 A6 | `entity_api/src/search/visibility.rs:36` | [rows] |
| SRCH-02 | GET /search?q=kumquat&types=actions | COACHEE | 200, A1 A2 A3 A4 A5 | `entity_api/src/search/visibility.rs:36`; `entity_api/src/search/action.rs:45` | [rows] no assignee narrowing |
| SRCH-03 | GET /search?q=kumquat&types=actions | OTHER_COACH | 200, A4 A5 | `entity_api/src/search/visibility.rs:36` | [rows] |
| SRCH-04 | GET /search?q=kumquat&types=actions | MEMBER, ADMIN, OUTSIDER | 200, no hits | `entity_api/src/search/visibility.rs:36`; `domain/src/search.rs:103` | [rows] Admin gets no org-wide search |
| SRCH-05 | GET /search?q=kumquat&types=actions | SUPER | 200, A1 to A6 | `entity_api/src/search/visibility.rs:26` | [rows] unrestricted |
| SRCH-06 | GET /search?q=kumquat&types=actions | REMOVED | 200, no hits | `entity/src/coaching_relationships.rs:131` | [rows] R3 dropped by the membership clause |
| SRCH-07 | GET /search?q=kumquat&types=actions&coaching_relationship_id=R1 | COACHEE | 200, A1 A2 A3 | `entity_api/src/search/visibility.rs:42` | [rows] filter intersects, never widens |
| SRCH-08 | GET /search?q=kumquat&types=actions&coaching_relationship_id=R1 | MEMBER | 200, no hits | `entity_api/src/search/visibility.rs:42` | [rows] not 403 |
| SRCH-09 | GET /search?q=kumquat&types=actions&organization_id=O1 | SUPER | 200, A1 to A6 | `entity_api/src/search/visibility.rs:39` | [rows] |
| SRCH-10 | GET /search?q=kumquat (all types) | COACHEE | 200, R1 and R2 hits of every type (S1 S2, G1 G2, A1 to A5, AG1, T1 T2, SG1 SG2) | `entity_api/src/search/visibility.rs:36` | [rows] notes are not searched |

### 4.12 Collaboration token (`JWT`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| JWT-01 | GET /jwt/generate_collab_token?coaching_session_id=S1 | COACH, COACHEE | 200, token for S1's document | `web/src/protect/jwt.rs:30` | |
| JWT-02 | GET /jwt/generate_collab_token?coaching_session_id=S1 | non-participants | 403 | `web/src/protect/jwt.rs:30` | Unknown session: 404 |
| JWT-03 | GET /jwt/generate_collab_token?coaching_session_id=S3 | REMOVED | 403 | `web/src/protect/jwt.rs:30` | |

### 4.13 Users (`USR`)

Rows here cover user routes that return relationship data and are not covered above
(`REL-13`, `REL-14`, `REL-17`, `SES-27` to `SES-32`, `ACT-18` to `ACT-26`, `GOAL-22` to
`GOAL-24`).

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| USR-01 | GET /users/{COACHEE_ID}/coaching_sessions?include=relationship | COACHEE | 200, S1 S2 SR1-* each with `relationship`, `coach`, `coachee` | `web/src/protect/users/coaching_sessions.rs:20`; `entity_api/src/coaching_session.rs:962` | [rows] |
| USR-02 | GET /users/{COACHEE_ID}/coaching-relationships?role=coach | COACHEE | 200, empty list | `entity_api/src/coaching_relationship.rs:551` | [rows] |
| USR-03 | GET /users/{COACH_ID}/coaching-relationships?role=coach | COACH | 200, R1 R3 | `entity_api/src/coaching_relationship.rs:551` | [rows] |

### 4.14 Organizations (`ORG`)

Organization membership changes decide `grants_access_to`; these rows create or end it.

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| ORG-01 | POST /organizations/O1/users `{..., coach_id: COACH}` | ADMIN, SUPER | 200 (201), new user plus relationship COACH to new user | `web/src/extractors/organization_admin_access.rs:42`; `domain/src/user.rs:156` | Sends MAIL-12 |
| ORG-02 | POST /organizations/O1/users | COACH, COACHEE, MEMBER, OUTSIDER | 403 | `web/src/extractors/organization_admin_access.rs:48` | |
| ORG-03 | POST /organizations/O1/users/{user}/role `{role: User, coach_id: COACH}` | ADMIN, SUPER | 200 (201), membership plus relationship | `web/src/controller/organization/user/role_controller.rs:80`; `domain/src/user_role.rs:77` | Target must be administrable by the caller, else 404. Sends MAIL-13 |
| ORG-04 | DELETE /organizations/O1/users/{REMOVED_ID}/role | ADMIN, SUPER | 200 (204) | `web/src/extractors/organization_admin_access.rs:42`; `domain/src/user_role.rs:107` | R3 is kept; REMOVED loses every `grants_access_to` route |
| ORG-05 | DELETE /organizations/O1/users/{user}/role | COACH, COACHEE, MEMBER, OUTSIDER | 403 | `web/src/extractors/organization_admin_access.rs:48` | |
| ORG-06 | DELETE /organizations/O1/users/{user} (single-org user, no relationship with a session) | ADMIN, SUPER | 200 (204) | `web/src/extractors/organization_admin_access.rs:42`; `domain/src/user.rs:237` | Deletes the account and every relationship it is in (their goals and series go too). Multi-org user: 409. With coaching history: ORG-08 |
| ORG-07 | DELETE /organizations/O1/users/{ADMIN_ID} | ADMIN | 403 | `web/src/controller/organization/user_controller.rs:188` | Cannot delete self |
| ORG-08 | DELETE /organizations/O1/users/{COACHEE_ID} | ADMIN, SUPER | 409 `user_has_coaching_history`, `details: {reason, user_id, relationship_count: 2}`; nothing deleted | `domain/src/user.rs:244`, `:253` | [membership] R1 and R2 have sessions. A session created mid-delete gets the same 409 without `relationship_count`. Remove from the organization instead (ORG-04) |

### 4.15 Real-time events (`SSE`)

`GET /sse` opens a stream for the caller; events go to the user ids each publisher computes.
Check with two browsers or `curl -N -b /tmp/<jar>.jar -H "$VER" $BASE/sse`.

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| SSE-01 | GET /sse | any authenticated caller | 200, event stream | `web/src/sse/handler.rs:12` | No relationship check; delivery is per user id |
| SSE-02 | POST /actions in S1 (assigned to COACH) | streams of COACH, COACHEE | both get `action_created` with the full action | `domain/src/action.rs:69`, `:83`; `entity_api/src/coaching_relationship.rs:194` | [rows] coachee receives coach-assigned actions here |
| SSE-03 | any action, goal, link, title or topic change in R3 | stream of REMOVED | nothing delivered | `entity_api/src/coaching_relationship.rs:194` | [rows] recipients filtered to current org members |
| SSE-04 | goal create/update in R1 | streams of COACH, COACHEE | both get the goal event | `domain/src/goal.rs:130` | [rows] |
| SSE-05 | transcript suggestions ready for S1 (JOB-03) | streams of COACH, COACHEE | both get `transcript_suggestions_changed` | `domain/src/transcript_suggestion/extract.rs:240` | [rows] |
| SSE-06 | POST /webhooks/recall_ai (recording or transcript update for S1) | streams of COACH, COACHEE | both get the recording/transcription event | `entity_api/src/coaching_session.rs:653` | [rows] webhook is signature-authenticated, no user |

## 5. Background jobs and emails

| ID | Trigger | Recipients today | Decided at (file:line) |
|---|---|---|---|
| JOB-01 | Session reminder sweep (`domain/src/jobs/session_reminder.rs:113`) | COACHEE only (the relationship's `coachee_id`), when still an org member and the session is in the window; coach never | `entity_api/src/coaching_session.rs:453`, `:464`, `:488`; `domain/src/emails.rs:1287` |
| JOB-02 | Pending reschedule settle (`domain/src/jobs/coaching_session_pending_reschedule.rs:122`), queued by a PUT that moves the schedule (`domain/src/coaching_session.rs:375`) | Participants still org members (MAIL-02); notice is stamped only when the coachee got it | `domain/src/emails.rs:1198`; `domain/src/jobs/coaching_session_pending_reschedule.rs:125` |
| JOB-03 | Transcript suggestions sweep (`domain/src/jobs/transcript_suggestions.rs:74`) | No email. SSE to participants still members (SSE-05). Prompt names coach and coachee | `domain/src/transcript_suggestion/extract.rs:240`, `:115` |
| JOB-04 | Note image purge (`domain/src/jobs/coaching_session_image_purge.rs`) | None | not relationship-scoped |
| JOB-05 | Password reset token cleanup (`domain/src/jobs/password_reset.rs`) | None | not relationship-scoped |
| MAIL-01 | Session scheduled: `POST /coaching_sessions` (`domain/src/coaching_session.rs:143`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:1007` |
| MAIL-02 | Session rescheduled (JOB-02) | COACH and COACHEE, each only if still an org member; none if both left | `domain/src/emails.rs:1198` |
| MAIL-03 | Session cancelled: `DELETE /coaching_sessions/{id}` (`domain/src/coaching_session.rs:536`); also orphan cancels in series edits (`domain/src/emails.rs:2640`) | COACH and COACHEE, members or not [membership]; nothing for a past session. In a series edit a removed participant gets only the cancel, not the reschedule | `domain/src/emails.rs:1612`, `:2623`, `:2640` |
| MAIL-04 | Session invite resend: SES-24 (`domain/src/coaching_session.rs:533`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2087` |
| MAIL-05 | Series scheduled: SER-01 (`domain/src/coaching_session_series.rs:141`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2160`, `:1953`, `:1977` |
| MAIL-06 | Series rescheduled: SER-08 (`domain/src/coaching_session_series.rs:290`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2203` |
| MAIL-07 | Series rescheduled from an occurrence on: SER-10 (`domain/src/coaching_session_series.rs:731`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2285` |
| MAIL-08 | Series cancelled: SER-12 (`domain/src/coaching_session_series.rs:872`) | COACH and COACHEE, members or not [membership] | `domain/src/emails.rs:2795` |
| MAIL-09 | Series invite resend: SER-14 (`domain/src/coaching_session_series.rs:884`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2037` |
| MAIL-10 | Session reminder (JOB-01) | COACHEE only | `domain/src/emails.rs:1246` |
| MAIL-11 | Action assigned: ACT-28, ACT-29, TRN-11, TRN-13 | Every listed assignee (the coach or coachee, per ACT rules) who is still an org member [membership] | `domain/src/emails.rs:2732`; `web/src/controller/action_controller.rs:81`, `:154`; `domain/src/transcript_suggestion/review.rs:377` |
| MAIL-12 | Welcome (magic link): ORG-01 and `POST /organizations/{id}/users/{user}/resend-invite` | The new user; `coach_first_name` is whoever sent the invite | `domain/src/emails.rs:242`, `:297` |
| MAIL-13 | Added to organization: ORG-03 | The attached user | `domain/src/emails.rs:312` |
| MAIL-14 | Password reset: `POST /password-reset/request` | The account owner | `domain/src/password_reset.rs:236`; not relationship-related |
