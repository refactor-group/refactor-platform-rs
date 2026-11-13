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
  action to someone who is no longer a member is refused (`assignee_not_a_member`); an
  assignee already on an action is not rechecked. Cancelling and deleting stay allowed.
  Rows: SES-33 to SES-36, SER-17 to SER-21, ACT-37 to ACT-39, TRN-28.
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
| `S4`, `S5` | A future session with no notes, actions or agreements, in R1 and R3. Created only for the delete rows (SES-21 to SES-23, SES-36), so no list row includes them |
| `SR1` | Series in R1 created by COACH (its sessions are `SR1-*`) |
| `SR3` | Series in R3 created by COACH before REMOVED is removed (its sessions are `SR3-*`); after phase M a series can't be created for a removed coachee |
| `A1` | Action in S1 assigned to COACH |
| `A2` | Action in S1 assigned to COACHEE |
| `A3` | Action in S1, unassigned |
| `A4` | Action in S2 assigned to OTHER_COACH |
| `A5` | Action in S2 assigned to COACHEE |
| `A6` | Action in S3 assigned to REMOVED |
| `A7` | Action in S3, unassigned |
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
# A1=... A2=... A3=... A4=... A5=... A6=... A7=... G1=... G2=... N1=... AG1=...
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
| REL-01 | GET /organizations/O1/coaching_relationships | COACH | 200, R1 R3 | `web/src/extractors/organization_member_access.rs:85`; `domain/src/coaching_relationship.rs:66` | [rows] participant filter |
| REL-02 | GET /organizations/O1/coaching_relationships | COACHEE | 200, R1 R2 | same as REL-01 | [rows] |
| REL-03 | GET /organizations/O1/coaching_relationships | OTHER_COACH | 200, R2 | same as REL-01 | [rows] |
| REL-04 | GET /organizations/O1/coaching_relationships | MEMBER | 200, empty list | same as REL-01 | [rows] |
| REL-05 | GET /organizations/O1/coaching_relationships | ADMIN | 200, R1 R2 R3 | `domain/src/coaching_relationship.rs:52`, `:61`; `entity_api/src/user.rs:236` | [rows] admin sees every relationship in the org |
| REL-06 | GET /organizations/O1/coaching_relationships | SUPER | 200, R1 R2 R3 | `web/src/extractors/organization_member_access.rs:81`; `domain/src/coaching_relationship.rs:61` | [rows] |
| REL-07 | GET /organizations/O1/coaching_relationships | OUTSIDER, REMOVED | 403 | `web/src/extractors/organization_member_access.rs:103` | Not a member of O1 |
| REL-08 | GET /organizations/O1/coaching_relationships/R1 | COACH, COACHEE | 200, R1 with names | `web/src/extractors/coaching_relationship_access.rs:84` | |
| REL-09 | GET /organizations/O1/coaching_relationships/R1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/extractors/coaching_relationship_access.rs:84` | ADMIN and SUPER list R1 (REL-05, REL-06) but cannot read it |
| REL-10 | GET /organizations/O2/coaching_relationships/R1 | COACH | 404 | `web/src/extractors/coaching_relationship_access.rs:78` | Path org must match the relationship's |
| REL-11 | GET /organizations/O1/coaching_relationships/R3 | REMOVED | 403 | `web/src/extractors/coaching_relationship_access.rs:84` | Participant, no longer a member |
| REL-12 | GET /organizations/O1/coaching_relationships/R3 | COACH | 200, R3 | `web/src/extractors/coaching_relationship_access.rs:84` | Coach keeps access after the coachee is removed. [membership] `coach_is_member` true, `coachee_is_member` false |
| REL-13 | GET /users/{COACHEE_ID}/coaching-relationships | COACHEE | 200, R1 R2 | `web/src/protect/users/mod.rs:26`; `entity_api/src/coaching_relationship.rs:671` | [rows] self only. `role=coach`: empty; `role=coachee`: R1 R2 |
| REL-14 | GET /users/{COACHEE_ID}/coaching-relationships | COACH, MEMBER, ADMIN, SUPER | 403 | `web/src/protect/users/mod.rs:26` | Self only, no admin bypass |
| REL-15 | POST /organizations/O1/coaching_relationships `{coach_id: COACH, coachee_id: MEMBER}` | ADMIN, SUPER | 200 (201), new relationship | `web/src/extractors/organization_admin_access.rs:42`; `entity_api/src/coaching_relationship.rs:74` | Same pair again returns the existing row. Both people must be O1 members |
| REL-16 | POST /organizations/O1/coaching_relationships | COACH, COACHEE, MEMBER, OUTSIDER | 403 | `web/src/extractors/organization_admin_access.rs:48` | |
| REL-17 | GET /users/{REMOVED_ID}/coaching-relationships | REMOVED | 200, empty list | `web/src/protect/users/mod.rs:26`; `entity_api/src/coaching_relationship.rs:671`, `:426` | [membership] self list leaves out R3; was 200, R3 |
| REL-18 | GET /organizations/O1/coaching_relationships | ADMIN | 200; R3 has `coach_is_member` true and `coachee_is_member` false, R1 and R2 have both true | `entity_api/src/coaching_relationship.rs:236` | [membership] Both fields are on every relationship object: org list, single read, `GET /users/{id}/coaching-relationships`, the create response (both true), and `include=relationship` on session reads |
| REL-19 | GET /organizations/{unknown}/coaching_relationships | COACH, ADMIN, SUPER | 404 | `web/src/extractors/organization_member_access.rs:59` | Checked before membership |
| REL-20 | POST /organizations/O1/coaching_relationships | OUTSIDER made an `Admin` of O2 | 403, nothing written | `web/src/extractors/organization_admin_access.rs:48` | An admin of another organization is not an admin of O1 |
| REL-21 | any relationship route | not signed in | 401 | `web/src/middleware/auth.rs:25` | |

### 4.2 Sessions (`SES`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| SES-01 | GET /coaching_sessions?coaching_relationship_id=R1&from_date=2000-01-01&to_date=2100-01-01 | COACH, COACHEE | 200, S1 and SR1-* | `web/src/protect/coaching_sessions.rs:33` | [rows] filtered by relationship id. `from_date` and `to_date` are required: 400 without them |
| SES-02 | GET /coaching_sessions?coaching_relationship_id=R1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/protect/coaching_sessions.rs:33` | Unknown relationship id: 404 (`:42`). Missing param: 400 |
| SES-03 | GET /coaching_sessions?coaching_relationship_id=R3 | REMOVED | 403 | `web/src/protect/coaching_sessions.rs:33` | |
| SES-04 | GET /coaching_sessions/S1 | COACH, COACHEE | 200, S1 plus `organization_id` O1 | `web/src/extractors/coaching_session_access.rs:118` | First read hydrates the session, see MEET-01 |
| SES-05 | GET /coaching_sessions/S1 | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | Unknown session: 404 (`:115`) |
| SES-06 | GET /coaching_sessions/S3 | REMOVED | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
| SES-07 | POST /coaching_sessions/S1/view | COACH, COACHEE | 200, prior marker | `web/src/extractors/coaching_session_access.rs:118` | Per-caller marker |
| SES-08 | POST /coaching_sessions/S1/view | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
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
| SES-19 | PATCH /coaching_sessions/S1/title | COACH, COACHEE | 200, S1 with new title | `web/src/extractors/coaching_session_access.rs:118` | Either participant |
| SES-20 | PATCH /coaching_sessions/S1/title | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
| SES-21 | DELETE /coaching_sessions/S4 | COACH | 200 (204) | `web/src/protect/coaching_sessions.rs:129` | Sends MAIL-03. A session with content: SES-37 |
| SES-22 | DELETE /coaching_sessions/S4 | COACHEE | 403 | `web/src/protect/coaching_sessions.rs:129` | |
| SES-23 | DELETE /coaching_sessions/S4 | non-participants | 403 | `web/src/protect/coaching_sessions.rs:129` | |
| SES-24 | POST /coaching_sessions/S1/resend_invite | COACH | 204 | `web/src/protect/coaching_sessions.rs:129` | 409 when past or a series member. Sends MAIL-04 |
| SES-25 | POST /coaching_sessions/S1/resend_invite | COACHEE | 403 | `web/src/protect/coaching_sessions.rs:129` | |
| SES-26 | POST /coaching_sessions/S1/resend_invite | non-participants | 403 | `web/src/protect/coaching_sessions.rs:129` | |
| SES-27 | GET /users/{COACH_ID}/coaching_sessions | COACH | 200, S1 S3 SR1-* SR3-* | `web/src/protect/users/coaching_sessions.rs:20`; `entity_api/src/coaching_session.rs:1159` | [rows] self only. `include=relationship` adds `coach`/`coachee` |
| SES-28 | GET /users/{COACHEE_ID}/coaching_sessions | COACHEE | 200, S1 S2 SR1-* | same as SES-27 | [rows] |
| SES-29 | GET /users/{COACHEE_ID}/coaching_sessions | COACH, MEMBER, ADMIN, SUPER | 403 | `web/src/protect/users/coaching_sessions.rs:20` | Same gate for `/counts` and `/nearest` |
| SES-30 | GET /users/{REMOVED_ID}/coaching_sessions | REMOVED | 200, empty list | `web/src/protect/users/coaching_sessions.rs:20` | [membership] self list leaves out S3; was 200, S3 |
| SES-31 | GET /users/{COACHEE_ID}/coaching_sessions/counts | COACHEE | 200, counts over S1 S2 SR1-* | `web/src/protect/users/coaching_sessions.rs:20`; `entity_api/src/coaching_session.rs:919` | [rows] |
| SES-32 | GET /users/{COACHEE_ID}/coaching_sessions/nearest?coaching_relationship_ids=R1,R2 | COACHEE | 200, nearest of R1 and of R2 | `web/src/protect/users/coaching_sessions.rs:20`; `entity_api/src/coaching_session.rs:1226` | [rows] a relationship the user is not in contributes nothing |
| SES-33 | POST /coaching_sessions `{coaching_relationship_id: R3}` | COACH | 409 `coachee_not_a_member`, nothing written, no email | `domain/src/coaching_session.rs:102` | [membership] `details: {reason, organization_id, user_id}` |
| SES-34 | PUT /coaching_sessions/S3 changing `date` or `duration_minutes` | COACH | 409 `coachee_not_a_member`, S3 unchanged | `domain/src/coaching_session.rs:390` | [membership] Other edits succeed (SES-18) |
| SES-35 | POST /coaching_sessions/S3/resend_invite | COACH | 409 `coachee_not_a_member`, no email | `domain/src/coaching_session.rs:565` | [membership] |
| SES-36 | DELETE /coaching_sessions/S5 | COACH | 200 (204) | `web/src/protect/coaching_sessions.rs:129` | [membership] Deleting stays allowed. Sends MAIL-03 to both COACH and REMOVED |
| SES-37 | DELETE /coaching_sessions/S1 | COACH | 409 `sessions_have_content`, `details: {reason, coaching_session_id}`; S1, its content and its notes document untouched, no email | `domain/src/coaching_session.rs:525` | S1 holds N1, AG1 and A1 to A3 |
| SES-38 | POST /coaching_sessions `{coaching_relationship_id: R1}` | COACH after removal from O1 | 403, nothing written, no email | `web/src/controller/coaching_session_controller.rs:186`; `web/src/extractors/coaching_relationship_access.rs:109` | [tightened] Restore the role afterwards |
| SES-39 | POST /coaching_sessions `{coaching_relationship_id: unknown}` | COACH | 404, nothing written, no email | `web/src/extractors/coaching_relationship_access.rs:105` | |
| SES-40 | GET /coaching_sessions/{unknown}, POST /coaching_sessions/{unknown}/resend_invite | COACH | 404, no email | `web/src/extractors/coaching_session_access.rs:115`; `web/src/protect/coaching_sessions.rs:125` | |
| SES-41 | PUT /coaching_sessions/S1 | COACH after removal from O1 | 403, S1 unchanged | `web/src/protect/coaching_sessions.rs:129` | [tightened] Restore the role afterwards |
| SES-42 | any /coaching_sessions route | not signed in | 401 | `web/src/middleware/auth.rs:25` | |

### 4.3 Series (`SER`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| SER-01 | POST /coaching_session_series `{coaching_relationship_id: R1}` | COACH | 200 (201), series and its sessions | `domain/src/coaching_session_series.rs:94` | Sends MAIL-05 |
| SER-02 | POST /coaching_session_series `{coaching_relationship_id: R1}` | COACHEE | 403 | `domain/src/coaching_session_series.rs:94` | |
| SER-03 | POST /coaching_session_series `{coaching_relationship_id: R1}` | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `domain/src/coaching_session_series.rs:94` | Unknown relationship: 404. [tightened] Coach check now also requires membership, see SER-16 |
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
| SER-22 | GET /coaching_session_series/{unknown}; POST its `/resend_invite`, `/reschedule` | COACH | 404, no email | `web/src/extractors/coaching_session_series_access.rs:165` | |
| SER-23 | any /coaching_session_series route | not signed in | 401 | `web/src/middleware/auth.rs:25` | |

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
| ACT-01 | GET /organizations/O1/coaching_relationships/actions | COACH | 200, `coachee_actions`: R1 [A1 A2 A3], R3 [A6 A7] | `web/src/extractors/organization_member_access.rs:85`; `entity_api/src/action.rs:615` | [rows] keyed by relationship id |
| ACT-02 | GET /organizations/O1/coaching_relationships/actions | COACHEE | 200, R1 [A2 A3], R2 [A5]; not A1, A4 | `entity_api/src/action.rs:482`, `:475` | [rows] coachee side sees own or unassigned |
| ACT-03 | GET /organizations/O1/coaching_relationships/actions | OTHER_COACH | 200, R2 [A4 A5] | `entity_api/src/action.rs:615` | [rows] |
| ACT-04 | GET /organizations/O1/coaching_relationships/actions | MEMBER, ADMIN, SUPER | 200, empty object | `web/src/controller/organization/coaching_relationship/actions_controller.rs:158` | [rows] only the caller's own relationships |
| ACT-05 | GET /organizations/O1/coaching_relationships/actions | OUTSIDER, REMOVED | 403 | `web/src/extractors/organization_member_access.rs:103` | |
| ACT-06 | GET /organizations/O1/coaching_relationships/actions?assignee=coach | COACHEE | 403 | `web/src/controller/organization/coaching_relationship/actions_controller.rs:169`, `:45` | Only when the caller coaches no relationship |
| ACT-07 | GET /organizations/O1/coaching_relationships/R1/actions | COACH | 200, A1 A2 A3 | `web/src/extractors/coaching_relationship_access.rs:84`; `web/src/controller/organization/coaching_relationship/actions_controller.rs:98` | [rows] |
| ACT-08 | GET /organizations/O1/coaching_relationships/R1/actions | COACHEE | 200, A2 A3; not A1 | `web/src/controller/organization/coaching_relationship/actions_controller.rs:98`; `entity_api/src/action.rs:578` | [rows] |
| ACT-09 | GET /organizations/O1/coaching_relationships/R1/actions?assignee=coach | COACHEE | 403 | `web/src/controller/organization/coaching_relationship/actions_controller.rs:100` | `assignee=coachee` or own id: 200, A2 |
| ACT-10 | GET /organizations/O1/coaching_relationships/R1/actions | MEMBER, OTHER_COACH, ADMIN, SUPER | 403 | `web/src/extractors/coaching_relationship_access.rs:84` | |
| ACT-11 | GET /organizations/O1/coaching_relationships/R1/actions | OUTSIDER | 403 | `web/src/extractors/organization_member_access.rs:103` | Member check runs first |
| ACT-12 | GET /organizations/O1/coaching_relationships/R3/actions | REMOVED | 403 | `web/src/extractors/organization_member_access.rs:103` | |
| ACT-13 | GET /organizations/O1/coaching_relationships/R3/actions | COACH | 200, A6 A7 | `web/src/extractors/coaching_relationship_access.rs:84` | [rows] |
| ACT-14 | GET /actions?coaching_session_id=S1 | COACH | 200, A1 A2 A3 | `web/src/protect/actions.rs:34` | [rows] |
| ACT-15 | GET /actions?coaching_session_id=S1 | COACHEE | 200, A1 A2 A3 | `web/src/protect/actions.rs:34`; `web/src/controller/action_controller.rs:356` | [rows] no assignee narrowing here |
| ACT-16 | GET /actions?coaching_session_id=S1 | non-participants | 403 | `web/src/protect/actions.rs:34` | |
| ACT-17 | GET /actions?coaching_session_id=S3 | REMOVED | 403 | `web/src/protect/actions.rs:34` | |
| ACT-18 | GET /users/{COACHEE_ID}/actions (scope=sessions, default) | COACHEE | 200, A1 A2 A3 A4 A5 | `web/src/protect/users/actions.rs:24`; `entity_api/src/action.rs:391` | [rows] self access; no assignee narrowing |
| ACT-19 | GET /users/{COACHEE_ID}/actions?scope=assigned | COACHEE | 200, A2 A5 | `web/src/protect/users/actions.rs:24`; `entity_api/src/action.rs:350` | [rows] |
| ACT-20 | GET /users/{COACH_ID}/actions (scope=sessions) | COACH | 200, A1 A2 A3 A6 A7 | `web/src/protect/users/actions.rs:24`; `entity_api/src/action.rs:391` | [rows] `scope=assigned`: A1 |
| ACT-21 | GET /users/{REMOVED_ID}/actions (scope=sessions) | REMOVED | 200, empty list | `web/src/protect/users/actions.rs:24`; `entity_api/src/action.rs:391` | [membership] self list leaves out A6; was 200, A6 |
| ACT-22 | GET /users/{COACHEE_ID}/actions (scope=sessions) | COACH | 200, A1 A2 A3; not A4 A5 | `web/src/protect/users/actions.rs:31`; `web/src/controller/user/action_controller.rs:85`; `entity_api/src/action.rs:393`; `entity_api/src/coaching_relationship.rs:416` | [rows] [tightened] only relationships where the caller coaches this user |
| ACT-23 | GET /users/{COACHEE_ID}/actions?scope=assigned | COACH | 200, A2; not A5 | `web/src/protect/users/actions.rs:31`; `web/src/controller/user/action_controller.rs:85`; `entity_api/src/action.rs:393`; `entity_api/src/coaching_relationship.rs:416` | [rows] [tightened] |
| ACT-24 | GET /users/{COACHEE_ID}/actions (scope=sessions) | OTHER_COACH | 200, A4 A5; not A1 A2 A3 | `web/src/protect/users/actions.rs:31`; `web/src/controller/user/action_controller.rs:85`; `entity_api/src/action.rs:393`; `entity_api/src/coaching_relationship.rs:416` | [rows] [tightened] |
| ACT-25 | GET /users/{COACHEE_ID}/actions | MEMBER, ADMIN, SUPER | 403 | `web/src/protect/users/actions.rs:31`; `web/src/controller/user/action_controller.rs:85`; `entity_api/src/action.rs:393`; `entity_api/src/coaching_relationship.rs:416` | [tightened] |
| ACT-26 | GET /users/{COACH_ID}/actions | COACHEE | 403 | `web/src/protect/users/actions.rs:31`; `web/src/controller/user/action_controller.rs:85`; `entity_api/src/action.rs:393`; `entity_api/src/coaching_relationship.rs:416` | [tightened] A coachee is not the coach's coach |
| ACT-26b | GET /users/{COACHEE_ID}/actions | COACH after their O1 role is removed | 403 | `web/src/protect/users/actions.rs:31`; `entity_api/src/coaching_relationship.rs:420` | [tightened] A coach must still be a member of the relationship's org; a coach still in another org sees only that org's relationships. Restore the role afterwards |
| ACT-27 | GET /actions/A1 | COACH, COACHEE | 200, A1 | `web/src/extractors/action_access.rs:50` | [tightened] |
| ACT-28 | POST /actions `{coaching_session_id: S1, assignee_ids: [COACHEE_ID]}` | COACH, COACHEE | 200 (201), new action | `web/src/controller/action_controller.rs:118` | [tightened] Sends MAIL-11 |
| ACT-29 | PUT /actions/A1 | COACH, COACHEE | 200, A1 updated | `web/src/extractors/action_access.rs:50` | [tightened] Newly added assignees get MAIL-11 |
| ACT-30 | PUT /actions/A1/status?value=completed | COACH, COACHEE | 200, A1 | `web/src/extractors/action_access.rs:50` | [tightened] |
| ACT-31 | DELETE /actions/A1 | COACH, COACHEE | 200, `{id}` | `web/src/extractors/action_access.rs:50` | [tightened] Run last in this table |
| ACT-32 | GET /actions/A1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/extractors/action_access.rs:50` | [tightened] Unknown action: 404 |
| ACT-33 | POST /actions `{coaching_session_id: S1}` | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, nothing written, no email | `web/src/controller/action_controller.rs:118` | [tightened] Unknown session: 404 |
| ACT-34 | PUT /actions/A1, PUT /actions/A1/status, DELETE /actions/A1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, A1 unchanged | `web/src/extractors/action_access.rs:50` | [tightened] |
| ACT-35 | GET, PUT, DELETE A6; POST `{coaching_session_id: S3}` | REMOVED | 403 | `web/src/extractors/action_access.rs:50`; `web/src/controller/action_controller.rs:118` | [tightened] |
| ACT-36 | POST /actions `{coaching_session_id: S1, assignee_ids: [MEMBER_ID]}`; PUT /actions/A3 adding MEMBER_ID | COACH | 422, nothing written, no email | `web/src/controller/action_controller.rs:61` (via `validate_links`, `:122`, `:247`) | [tightened] Assignees must be the relationship's coach or coachee |
| ACT-37 | POST /actions `{coaching_session_id: S3, assignee_ids: [REMOVED_ID]}` | COACH | 409 `assignee_not_a_member`, nothing written, no email | `web/src/controller/action_controller.rs:75`; `domain/src/coaching_relationship.rs:105` | [membership] `details: {reason, organization_id, user_id}` |
| ACT-38 | PUT /actions/A6 editing the body, REMOVED still assigned | COACH | 200, A6 updated, no email | `web/src/controller/action_controller.rs:75` | [membership] Only newly added assignees are checked |
| ACT-39 | PUT /actions/A7 adding REMOVED_ID as an assignee | COACH | 409 `assignee_not_a_member`, A7 unchanged, no email | `web/src/controller/action_controller.rs:75`; `domain/src/coaching_relationship.rs:117` | [membership] `details: {reason, organization_id, user_id}` |
| ACT-40 | POST /actions `{coaching_session_id: S1, goal_id: G2}` | COACH | 422, nothing written | `web/src/controller/action_controller.rs:81` | [tightened] G2 belongs to R2; an action's goal must be in its relationship |

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
| GOAL-13 | POST /coaching_sessions/SR1-1/goals `{goal_id: G1}` (an R1 session G1 isn't linked to yet) | COACH, COACHEE | 200 (201), link | `web/src/controller/coaching_session/goal_controller.rs:47` | [tightened] Non-participants: GOAL-26 |
| GOAL-14 | DELETE /coaching_sessions/S1/goals/G1 | COACH, COACHEE | 200, `{goal_id}` | `web/src/controller/coaching_session/goal_controller.rs:97` | [tightened] Non-participants: GOAL-26 |
| GOAL-15 | GET /coaching_sessions/goals?coaching_relationship_id=R1 | COACH, COACHEE | 200, `session_goals` S1 [G1], SR1-* [] | `web/src/protect/goals.rs:154` | [rows] |
| GOAL-16 | GET /coaching_sessions/goals?coaching_relationship_id=R1 | non-participants | 403 | `web/src/protect/goals.rs:154` | |
| GOAL-17 | GET /coaching_sessions/goals?coaching_session_ids=S1,S2 | COACH | 403 | `web/src/protect/goals.rs:141` | [tightened] Every id is authorized; S2 is in R2. `S1` alone: 200, S1 [G1] |
| GOAL-18 | GET /coaching_sessions/goals?coaching_session_ids=S1 | non-participants | 403 | `web/src/protect/goals.rs:184` | |
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
| TOP-01 | GET /coaching_sessions/S1/topics | COACH, COACHEE | 200, T1 T2 | `web/src/extractors/coaching_session_access.rs:118` | [rows] |
| TOP-02 | GET /coaching_sessions/S1/topics | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
| TOP-03 | POST /coaching_sessions/S1/topics | COACH, COACHEE | 200 (201), new topic authored by caller | `web/src/extractors/coaching_session_access.rs:118` | |
| TOP-04 | POST /coaching_sessions/S1/topics | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
| TOP-05 | PATCH /coaching_sessions/S1/topics/reorder | COACH, COACHEE | 200, T1 T2 reordered | `web/src/extractors/coaching_session_access.rs:118` | |
| TOP-06 | PUT /coaching_sessions/S1/topics/T2 | COACH | 200, T2 | `web/src/extractors/coaching_session_topic_access.rs:46` | Any participant edits any topic |
| TOP-07 | PUT /coaching_sessions/S1/topics/T1 | COACHEE | 200, T1 | `web/src/extractors/coaching_session_topic_access.rs:46` | |
| TOP-08 | PUT /coaching_sessions/S1/topics/T1 | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:37` | Topic not in S1: 404 |
| TOP-09 | DELETE /coaching_sessions/S1/topics/T2 | COACH | 200 | `web/src/extractors/coaching_session_topic_access.rs:90` | Coach deletes any topic |
| TOP-10 | DELETE /coaching_sessions/S1/topics/T2 | COACHEE | 200 | `web/src/extractors/coaching_session_topic_access.rs:78` | Author |
| TOP-11 | DELETE /coaching_sessions/S1/topics/T1 | COACHEE | 404 | `web/src/extractors/coaching_session_topic_access.rs:94` | Not author, not coach |
| TOP-12 | DELETE /coaching_sessions/S1/topics/T1 | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:72` | |
| TOP-16 | PATCH /coaching_sessions/S1/topics/T1/status | COACH, COACHEE | 200, T1 | `web/src/extractors/coaching_session_topic_access.rs:46` | Either participant |
| TOP-17 | PATCH /coaching_sessions/S1/topics/T1/status | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:37` | |
| TOP-18 | POST /coaching_sessions/S1/topics/T2/undo (T2 deleted by COACH) | COACH | 404 | `web/src/extractors/coaching_session_topic_access.rs:137` | Undoing a delete is author only |
| TOP-19 | POST /coaching_sessions/S1/topics/T2/undo (T2 deleted) | COACHEE | 200, T2 restored | `web/src/extractors/coaching_session_topic_access.rs:137` | |
| TOP-20 | POST /coaching_sessions/{dest}/topics/T1/undo (T1 deferred) | COACH, COACHEE | 200, T1 back in S1 | `web/src/extractors/coaching_session_topic_access.rs:134` | Defer undo open to either participant |
| TOP-21 | POST /coaching_sessions/S1/topics/T1/undo | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:120` | |

### 4.9 Transcripts, recordings and images (`TRN`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| TRN-01 | GET /coaching_sessions/S1/transcriptions | COACH, COACHEE | 200, TR1 with `speakers` | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-02 | GET /coaching_sessions/S1/transcriptions | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-03 | GET /coaching_sessions/S1/transcriptions/TR1 (`Accept: text/plain`, `speaker=coach`) | COACH, COACHEE | 200, coach lines only | `web/src/extractors/coaching_session_access.rs:118`; `domain/src/transcription.rs:579` | 422 if no speaker attributed to the role |
| TRN-04 | GET /coaching_sessions/S1/transcriptions/TR1 | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | TR1 under another session: 404 |
| TRN-05 | GET /coaching_sessions/S1/transcriptions/TR1/transcription_segments | COACH, COACHEE | 200, TR1 segments | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-06 | GET /coaching_sessions/S1/transcriptions/TR1/transcription_segments | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-07 | GET /coaching_sessions/S1/transcript_suggestions | COACH, COACHEE | 200, SG1 SG2 | `web/src/extractors/coaching_session_access.rs:118`; `domain/src/transcript_suggestion/review.rs:246` | [rows] coachee also sees coach-only SG1 |
| TRN-08 | GET /coaching_sessions/S1/transcript_suggestions | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-09 | GET, POST /coaching_sessions/S1/transcript_suggestions/scan | COACH, COACHEE | GET 200, POST 202; scan status | `web/src/extractors/coaching_session_access.rs:118` | POST: 409 if running or done |
| TRN-10 | GET, POST /coaching_sessions/S1/transcript_suggestions/scan | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-11 | POST /coaching_sessions/S1/transcript_suggestions/SG1/accept | COACH | 200 (201), new action | `domain/src/transcript_suggestion/review.rs:333`, `:553`, `:202` | Coach may always act. Sends MAIL-11 |
| TRN-12 | POST /coaching_sessions/S1/transcript_suggestions/SG1/accept | COACHEE | 403 | `domain/src/transcript_suggestion/review.rs:206` | Coach-only suggestion |
| TRN-13 | POST /coaching_sessions/S1/transcript_suggestions/SG2/accept | COACHEE | 200 (201), new action | `domain/src/transcript_suggestion/review.rs:206` | Edits that add or remove the coach: 403 |
| TRN-14 | POST /coaching_sessions/S1/transcript_suggestions/SG1/accept | non-participants | 404 | `web/src/extractors/transcript_suggestion_access.rs:40` | All failures collapse to 404 |
| TRN-15 | POST .../SG1/dismiss, .../SG1/restore | COACH | 204 | `domain/src/transcript_suggestion/review.rs:438`, `:553` | |
| TRN-16 | POST .../SG1/dismiss, .../SG1/restore | COACHEE | 403 | `domain/src/transcript_suggestion/review.rs:206` | SG2: 204 |
| TRN-17 | POST .../SG1/dismiss, .../SG1/restore | non-participants | 404 | `web/src/extractors/transcript_suggestion_access.rs:40` | |
| TRN-18 | GET /coaching_sessions/S1/meeting_recording | COACH, COACHEE | 200, latest recording or null | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-19 | POST /coaching_sessions/S1/meeting_recording `{meeting_url}` | COACH, COACHEE | 200 (201), recording | `web/src/extractors/coaching_session_access.rs:118` | Either participant starts a bot; 409 if one is active |
| TRN-20 | DELETE /coaching_sessions/S1/meeting_recording | COACH, COACHEE | 200 | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-21 | GET, POST, DELETE /coaching_sessions/S1/meeting_recording | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-22 | GET, POST, DELETE /coaching_sessions/S3/meeting_recording | REMOVED | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-23 | POST /coaching_sessions/S1/images | COACH, COACHEE | 200 (201), image | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-24 | POST /coaching_sessions/S1/images | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:118` | |
| TRN-25 | GET /coaching_session_images/IMG1 | COACH, COACHEE | 302 to storage, or 200 bytes | `web/src/extractors/coaching_session_image_access.rs:54` | |
| TRN-26 | DELETE, POST restore /coaching_session_images/IMG1 | COACH, COACHEE | 200, IMG1 | `web/src/extractors/coaching_session_image_access.rs:54` | Either participant, not only the uploader |
| TRN-27 | GET, DELETE /coaching_session_images/IMG1 | non-participants | 403 | `web/src/extractors/coaching_session_image_access.rs:54` | Unknown image: 404 |
| TRN-28 | POST /coaching_sessions/S3/transcript_suggestions/SG3/accept | COACH | 409 `assignee_not_a_member`, no action written, no email | `domain/src/transcript_suggestion/review.rs:339` | [membership] |

### 4.10 Meeting links and OAuth-dependent flows (`MEET`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| MEET-01 | GET /coaching_sessions/SR1-x (not yet hydrated) | COACHEE | 200, session with provider and `meeting_url` from COACH's connection | `web/src/extractors/coaching_session_access.rs:118`; `domain/src/coaching_session.rs:268` | Hydration always uses the coach's account, whoever reads first |
| MEET-02 | POST /coaching_sessions with `provider` | caller per SES-09 to SES-14 | as SES-09 to SES-14; a link is attached only on SES-09 | `domain/src/coaching_session.rs:116` | [tightened] Link comes from the coach's account or a reused relationship link (`:613`) |
| MEET-03 | PUT /coaching_sessions/S1 `{meeting_url, provider}` | COACH | 200 (204) | `web/src/protect/coaching_sessions.rs:129` | Coach only; COACHEE 403 (SES-16) |
| MEET-04 | POST /coaching_sessions/S1/meeting_recording `{meeting_url}` | COACHEE | 200 (201) | `web/src/extractors/coaching_session_access.rs:118` | URL is caller supplied; not tied to an account |
| MEET-05 | GET /oauth/connections, GET/DELETE /oauth/connections/{provider} | any authenticated caller | 200, caller's own connections | `web/src/router.rs:904`, `:906` (auth only) | Self scoped, no relationship data. Listed so MEET-01 has its prerequisite |

### 4.11 Search (`SRCH`)

All rows: `GET /search?q=kumquat&types=actions`. Rows are scoped by
`entity/src/coaching_relationships.rs:124` (`visible_to`: participant AND member of the
relationship's org) via `entity_api/src/search/visibility.rs:18`; SuperAdmins skip it.

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| SRCH-01 | GET /search?q=kumquat&types=actions | COACH | 200, A1 A2 A3 A6 A7 | `entity_api/src/search/visibility.rs:36` | [rows] Automated: SQL-shape mock and visible-relationship SQLite only; check hits by hand. |
| SRCH-02 | GET /search?q=kumquat&types=actions | COACHEE | 200, A1 A2 A3 A4 A5 | `entity_api/src/search/visibility.rs:36`; `entity_api/src/search/action.rs:45` | [rows] no assignee narrowing. Automated: SQL-shape mock and visible-relationship SQLite only; check hits by hand. |
| SRCH-03 | GET /search?q=kumquat&types=actions | OTHER_COACH | 200, A4 A5 | `entity_api/src/search/visibility.rs:36` | [rows] Automated: SQL-shape mock and visible-relationship SQLite only; check hits by hand. |
| SRCH-04 | GET /search?q=kumquat&types=actions | MEMBER, ADMIN, OUTSIDER | 200, no hits | `entity_api/src/search/visibility.rs:36`; `domain/src/search.rs:103` | [rows] Admin gets no org-wide search |
| SRCH-05 | GET /search?q=kumquat&types=actions | SUPER | 200, A1 to A7 | `entity_api/src/search/visibility.rs:26` | [rows] unrestricted. Automated: SQL-shape mock and visible-relationship SQLite only; check hits by hand. |
| SRCH-06 | GET /search?q=kumquat&types=actions | REMOVED | 200, no hits | `entity/src/coaching_relationships.rs:131` | [rows] R3 dropped by the membership clause |
| SRCH-07 | GET /search?q=kumquat&types=actions&coaching_relationship_id=R1 | COACHEE | 200, A1 A2 A3 | `entity_api/src/search/visibility.rs:42` | [rows] filter intersects, never widens. Automated: SQL-shape mock and visible-relationship SQLite only; check hits by hand. |
| SRCH-08 | GET /search?q=kumquat&types=actions&coaching_relationship_id=R1 | MEMBER | 200, no hits | `entity_api/src/search/visibility.rs:42` | [rows] not 403 |
| SRCH-09 | GET /search?q=kumquat&types=actions&organization_id=O1 | SUPER | 200, A1 to A7 | `entity_api/src/search/visibility.rs:39` | [rows] Automated: SQL-shape mock and visible-relationship SQLite only; check hits by hand. |
| SRCH-10 | GET /search?q=kumquat (all types) | COACHEE | 200, R1 and R2 hits of every type (S1 S2, G1 G2, A1 to A5, AG1, T1 T2, SG1 SG2) | `entity_api/src/search/visibility.rs:36` | [rows] notes are not searched. Automated: SQL-shape mock and visible-relationship SQLite only; check hits by hand. |

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
| USR-01 | GET /users/{COACHEE_ID}/coaching_sessions?include=relationship | COACHEE | 200, S1 S2 SR1-* each with `relationship`, `coach`, `coachee` | `web/src/protect/users/coaching_sessions.rs:20`; `entity_api/src/coaching_session.rs:1159`, `:1347` | [rows] |
| USR-02 | GET /users/{COACHEE_ID}/coaching-relationships?role=coach | COACHEE | 200, empty list | `entity_api/src/coaching_relationship.rs:662` | [rows] |
| USR-03 | GET /users/{COACH_ID}/coaching-relationships?role=coach | COACH | 200, R1 R3 | `entity_api/src/coaching_relationship.rs:662` | [rows] |
| USR-04 | any /users route | not signed in | 401 | `web/src/middleware/auth.rs:25` | |
| USR-05 | PUT /users/{COACHEE_ID} `{working_hours_start}` | COACH, MEMBER, ADMIN, SUPER | 403, COACHEE unchanged | `web/src/protect/users/mod.rs:47` | Self only |
| USR-06 | GET /users?email={MEMBER email} | COACH, COACHEE, MEMBER, OUTSIDER | 403 | `web/src/controller/user_controller.rs:57` | Only an org `Admin` or SUPER may look users up |
| USR-07 | GET /users?email={MEMBER email} | ADMIN | 200, MEMBER | `domain/src/user_role.rs:279`; `entity_api/src/user_role.rs:331` | [rows] the lookup's rate limit takes a Postgres advisory lock, so the route is checked by hand and the scope in `domain` |
| USR-08 | GET /users?email={OUTSIDER email} | ADMIN | 200, empty list, also once OUTSIDER has role history in O2 | `domain/src/user_role.rs:279`; `entity_api/src/user_role.rs:331`; `entity_api/src/user_role_change.rs:58` | [rows] Not found and not visible look the same |
| USR-09 | GET /users?email={REMOVED email} after ORG-04 | ADMIN | 200, REMOVED; ADMIN may also attach them (ORG-03) | `domain/src/user_role.rs:279`, `:297`; `entity_api/src/user_role_change.rs:58` | [rows] Without O1 role history: empty list |
| USR-10 | GET /users?email={any fixture email} | SUPER | 200, that user | `domain/src/user_role.rs:279` | [rows] |

### 4.14 Organizations (`ORG`)

Organization membership changes decide `grants_access_to`; these rows create or end it.

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| ORG-01 | POST /organizations/O1/users `{..., coach_id: COACH}` | ADMIN, SUPER | 200 (201), new user plus relationship COACH to new user | `web/src/extractors/organization_admin_access.rs:42`; `domain/src/user.rs:156` | Sends MAIL-12 |
| ORG-02 | POST /organizations/O1/users | COACH, COACHEE, MEMBER, OUTSIDER | 403 | `web/src/extractors/organization_admin_access.rs:48` | |
| ORG-03 | POST /organizations/O1/users/{user}/role `{role: User, coach_id: COACH}` | ADMIN, SUPER | 200 (201), membership plus relationship | `web/src/controller/organization/user/role_controller.rs:80`; `domain/src/user_role.rs:77` | Target must be administrable by the caller, else 404. Sends MAIL-13 |
| ORG-04 | DELETE /organizations/O1/users/{REMOVED_ID}/role | ADMIN, SUPER | 200 (204) | `web/src/extractors/organization_admin_access.rs:42`; `domain/src/user_role.rs:107` | R3 is kept; REMOVED loses every `grants_access_to` route |
| ORG-05 | DELETE /organizations/O1/users/{user}/role | COACH, COACHEE, MEMBER, OUTSIDER | 403 | `web/src/extractors/organization_admin_access.rs:48` | |
| ORG-06 | DELETE /organizations/O1/users/{user} (single-org user, no relationship with a session) | ADMIN, SUPER | 200 (204) | `web/src/extractors/organization_admin_access.rs:42`; `domain/src/user.rs:249` | Deletes the account and every relationship it is in (their goals and series go too). Multi-org user: 409. With coaching history: ORG-08 |
| ORG-07 | DELETE /organizations/O1/users/{ADMIN_ID} | ADMIN | 403 | `web/src/controller/organization/user_controller.rs:183` | Cannot delete self |
| ORG-08 | DELETE /organizations/O1/users/{COACHEE_ID} | ADMIN, SUPER | 409 `user_has_coaching_history`, `details: {reason, user_id, relationship_count: 2}`; nothing deleted | `domain/src/user.rs:244`, `:253` | [membership] R1 and R2 have sessions. A session created mid-delete gets the same 409 without `relationship_count`. Remove from the organization instead (ORG-04) |
| ORG-09 | POST /organizations/O1/users/{user}/resend-invite | COACH, COACHEE, MEMBER, OUTSIDER | 403, no email | `web/src/extractors/organization_admin_access.rs:48` | |
| ORG-10 | POST /organizations/O1/users/{user}/resend-invite (an O1 member who never set a password) | ADMIN, SUPER | 200, the user; sends MAIL-12 | `web/src/extractors/organization_admin_access.rs:42`; `web/src/extractors/organization_user_access.rs:62`; `web/src/controller/organization/user_controller.rs:140` | A user with a password: 422 |
| ORG-11 | DELETE /organizations/O1/users/{MEMBER_ID} | COACH, COACHEE, MEMBER, OUTSIDER | 403, MEMBER kept | `web/src/extractors/organization_admin_access.rs:48` | |
| ORG-12 | POST /organizations/O1/users; POST `.../users/{user}/resend-invite`; DELETE `.../users/{user}`; GET, POST, PUT, DELETE `.../users/{user}/role` | OUTSIDER made an `Admin` of O2 | 403 for MEMBER, an unknown user or an unknown organization alike; nothing changed, no email | `web/src/extractors/organization_admin_access.rs:48` | |
| ORG-13 | POST /organizations/{unknown}/users | SUPER | 404, nothing written | `web/src/extractors/organization_admin_access.rs:55` | |
| ORG-14 | POST /organizations/{O2 or unknown}/users | MEMBER, ADMIN | 403 for both, same body | `web/src/extractors/organization_admin_access.rs:48` | The admin check runs before the lookup, so a non-admin cannot tell which organizations exist |
| ORG-15 | DELETE /organizations/O1/users/{OUTSIDER_ID or unknown} | ADMIN, SUPER | 404, OUTSIDER kept | `web/src/extractors/organization_user_access.rs:66` | The target must hold a role in the path organization |
| ORG-16 | POST /organizations/O1/users/{OUTSIDER_ID}/role `{role: User}` | ADMIN | 404, no role written, no email | `web/src/controller/organization/user/role_controller.rs:80`; `domain/src/user_role.rs:297` | A user the caller may not administer looks missing |
| ORG-17 | GET /organizations/O1/users/{OUTSIDER_ID or unknown}/role | ADMIN | 404 for both, same body | `domain/src/user_role.rs:156` | |
| ORG-18 | POST /organizations/O1/users/{OUTSIDER_ID}/role `{role: User}` | ADMIN made an `Admin` of O2 too | 200 (201), OUTSIDER holds `User` in O1 | `web/src/controller/organization/user/role_controller.rs:80`; `domain/src/user_role.rs:297`; `entity_api/src/user_role.rs:331` | An admin may attach a current member of another org they administer |
| ORG-19 | POST /organizations/O1/users/{MEMBER_ID}/role | ADMIN, SUPER | 409, MEMBER keeps `User` | `web/src/controller/organization/user/role_controller.rs:80`; `domain/src/user_role.rs:70` | MEMBER is visible, so the answer is the conflict, not 404 |
| ORG-20 | DELETE /organizations/O1/users/{ADMIN_ID}/role | ADMIN | 403, ADMIN keeps `Admin` | `web/src/controller/organization/user/role_controller.rs:252` | Cannot remove self |
| ORG-21 | GET, PUT /organizations/O1/users/{ADMIN_ID or unknown}/role | COACH, COACHEE, MEMBER, OUTSIDER | 403 for both, same body; ADMIN unchanged | `web/src/extractors/organization_admin_access.rs:48` | |
| ORG-22 | PUT /organizations/O1/users/{ADMIN_ID}/role `{role: Admin or User}` | ADMIN | 403, ADMIN keeps `Admin` | `web/src/controller/organization/user/role_controller.rs:196` | Cannot change own role |
| ORG-23 | GET /organizations/O1/users/{MEMBER_ID}/role | ADMIN, SUPER | 200, `User` in O1 for MEMBER | `web/src/extractors/organization_admin_access.rs:42`; `domain/src/user_role.rs:156` | |
| ORG-24 | PUT /organizations/O1/users/{MEMBER_ID}/role `{role: Admin}` | ADMIN, SUPER | 200, MEMBER holds `Admin` | `web/src/extractors/organization_admin_access.rs:42`; `web/src/controller/organization/user/role_controller.rs:173` | SUPER holds no O1 role |
| ORG-25 | POST /organizations `{name}` | SUPER | 200 (201), new organization | `web/src/extractors/super_admin_access.rs:27` | Same gate: PUT, DELETE `/organizations/{id}`, POST `/archive`, `/unarchive` |
| ORG-26 | POST /organizations | ADMIN, COACH, MEMBER, OUTSIDER | 403, nothing written | `web/src/extractors/super_admin_access.rs:29` | |
| ORG-27 | POST /organizations | OUTSIDER holding `SuperAdmin` scoped to O2 | 403, nothing written | `web/src/extractors/super_admin_access.rs:27`; `entity/src/users.rs:95` | Only a `SuperAdmin` role with no organization counts. The model refuses to save a scoped one, so set it in the database by hand |
| ORG-28 | any /organizations route | not signed in | 401 | `web/src/middleware/auth.rs:25` | |

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
| SSE-06 | POST /webhooks/recall_ai (recording or transcript update for S1) | streams of COACH, COACHEE | both get the recording/transcription event | `entity_api/src/coaching_session.rs:655` | [rows] webhook is signature-authenticated, no user |

## 5. Background jobs and emails

| ID | Trigger | Recipients today | Decided at (file:line) | Notes |
|---|---|---|---|---|
| JOB-01 | Session reminder sweep (`domain/src/jobs/session_reminder.rs:113`) | COACHEE only (the relationship's `coachee_id`), when still an org member and the session is in the window; coach never | `entity_api/src/coaching_session.rs:466`, `:473`, `:487`; `domain/src/emails.rs:1363` | Automated: SQL-shape mock only; check recipients by hand. |
| JOB-02 | Pending reschedule settle (`domain/src/jobs/coaching_session_pending_reschedule.rs:122`), queued by a PUT that moves the schedule (`domain/src/coaching_session.rs:405`) | Participants still org members (MAIL-02); notice is stamped only when the coachee got it | `domain/src/emails.rs:1286`; `domain/src/jobs/coaching_session_pending_reschedule.rs:125` | |
| JOB-03 | Transcript suggestions sweep (`domain/src/jobs/transcript_suggestions.rs:74`) | No email. SSE to participants still members (SSE-05). Prompt names coach and coachee | `domain/src/transcript_suggestion/extract.rs:240`, `:115` | |
| JOB-04 | Note image purge (`domain/src/jobs/coaching_session_image_purge.rs`) | None | not relationship-scoped | |
| JOB-05 | Password reset token cleanup (`domain/src/jobs/password_reset.rs`) | None | not relationship-scoped | |
| MAIL-01 | Session scheduled: `POST /coaching_sessions` (`domain/src/coaching_session.rs:149`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:1056` | |
| MAIL-02 | Session rescheduled (JOB-02) | COACH and COACHEE, each only if still an org member; none if both left | `domain/src/emails.rs:1286` | |
| MAIL-03 | Session cancelled: `DELETE /coaching_sessions/{id}` (`domain/src/coaching_session.rs:536`); also orphan cancels in series edits (`domain/src/emails.rs:2640`) | COACH and COACHEE, members or not [membership]; nothing for a past session. In a series edit a removed participant gets only the cancel, not the reschedule | `domain/src/emails.rs:1612`, `:2623`, `:2640` | |
| MAIL-04 | Session invite resend: SES-24 (`domain/src/coaching_session.rs:570`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2135` | |
| MAIL-05 | Series scheduled: SER-01 (`domain/src/coaching_session_series.rs:145`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2224`, `:2041` | |
| MAIL-06 | Series rescheduled: SER-08 (`domain/src/coaching_session_series.rs:308`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2278` | |
| MAIL-07 | Series rescheduled from an occurrence on: SER-10 (`domain/src/coaching_session_series.rs:754`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2366` | |
| MAIL-08 | Series cancelled: SER-12 (`domain/src/coaching_session_series.rs:872`) | COACH and COACHEE, members or not [membership] | `domain/src/emails.rs:2795` | |
| MAIL-09 | Series invite resend: SER-14 (`domain/src/coaching_session_series.rs:932`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2099` | |
| MAIL-10 | Session reminder (JOB-01) | COACHEE only | `domain/src/emails.rs:1363`, `:1367` | Automated: SQL-shape mock only; check recipients by hand. |
| MAIL-11 | Action assigned: ACT-28, ACT-29, TRN-11, TRN-13 | Every listed assignee (the coach or coachee, per ACT rules) who is still an org member [membership] | `domain/src/emails.rs:2873`; `web/src/controller/action_controller.rs:134`, `:197`; `domain/src/transcript_suggestion/review.rs:380` | |
| MAIL-12 | Welcome (magic link): ORG-01 and `POST /organizations/{id}/users/{user}/resend-invite` | The new user; `coach_first_name` is whoever sent the invite | `domain/src/emails.rs:249`, `:297` | |
| MAIL-13 | Added to organization: ORG-03 | The attached user | `domain/src/emails.rs:319` | |
| MAIL-14 | Password reset: `POST /password-reset/request` | The account owner | `domain/src/password_reset.rs:236`; not relationship-related | |

## 6. Automated checks

Where each row is checked: a route test on in-memory SQLite (a `*_sqlite_tests.rs`
file beside the module that serves the route, which names the row in a `/// <ID>` doc comment),
a frozen `domain` test, or by hand with the reason.

| Row | Checked by |
|---|---|
| REL-01 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_coach_lists_only_their_own_relationships` |
| REL-02 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_coachee_lists_only_their_own_relationships` |
| REL-03 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::another_coach_lists_only_their_relationship` |
| REL-04 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_member_in_no_relationship_lists_none` |
| REL-05 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::an_admin_lists_every_relationship_in_the_org` |
| REL-06 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_super_admin_lists_every_relationship_in_the_org` |
| REL-07 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::non_members_cannot_list_the_orgs_relationships` |
| REL-08 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::participants_read_their_relationship_with_names` |
| REL-09 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::non_participants_cannot_read_a_relationship` |
| REL-10 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_relationship_under_another_org_is_not_found` |
| REL-11 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_removed_coachee_cannot_read_their_relationship` |
| REL-12 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_coach_keeps_a_relationship_whose_coachee_left` |
| REL-13 | `web/src/controller/user/coaching_relationships_controller_sqlite_tests.rs::a_coachee_lists_their_own_relationships_by_role` |
| REL-14 | `web/src/controller/user/coaching_relationships_controller_sqlite_tests.rs::only_the_user_lists_their_own_relationships` |
| REL-15 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::admins_create_a_relationship_once_per_pair` |
| REL-16 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::non_admins_cannot_create_a_relationship` |
| REL-17 | `web/src/controller/user/coaching_relationships_controller_sqlite_tests.rs::a_removed_coachee_lists_none_of_their_own_relationships`; `domain/src/coaching_relationship_sqlite_tests.rs::a_users_own_relationships_skip_an_org_they_left` |
| REL-18 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::an_admin_sees_who_is_still_a_member` |
| REL-19 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::an_unknown_organizations_relationships_are_not_found` |
| REL-20 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::an_admin_of_another_org_cannot_create_a_relationship` |
| REL-21 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::signed_out_callers_cannot_reach_relationships` |
| SES-01 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::participants_list_the_relationships_sessions` |
| SES-02 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::non_participants_cannot_list_the_relationships_sessions` |
| SES-03 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_removed_coachee_cannot_list_their_relationships_sessions` |
| SES-04 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::participants_read_a_session_with_its_organization` |
| SES-05 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::non_participants_cannot_read_a_session` |
| SES-06 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_removed_coachee_cannot_read_their_session` |
| SES-07 | by hand: the view upsert is Postgres-only SQL |
| SES-08 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::non_participants_cannot_mark_a_session_viewed` |
| SES-09 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_creates_a_session_with_its_notes_document` |
| SES-10 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coachee_cannot_create_a_session` |
| SES-11 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::members_outside_the_relationship_cannot_create_a_session` |
| SES-12 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::admins_cannot_create_a_session` |
| SES-13 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::an_outsider_cannot_create_a_session` |
| SES-14 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_removed_coachee_cannot_create_a_session` |
| SES-15 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_updates_a_session` |
| SES-16 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coachee_cannot_update_a_session` |
| SES-17 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::non_participants_cannot_update_a_session` |
| SES-18 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_retitles_a_session_whose_coachee_left` |
| SES-19 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::either_participant_retitles_a_session` |
| SES-20 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::non_participants_cannot_retitle_a_session` |
| SES-21 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_deletes_a_content_free_session` |
| SES-22 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coachee_cannot_delete_a_session` |
| SES-23 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::non_participants_cannot_delete_a_session` |
| SES-24 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_resends_a_sessions_invite` |
| SES-25 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coachee_cannot_resend_a_sessions_invite` |
| SES-26 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::non_participants_cannot_resend_a_sessions_invite` |
| SES-27 | `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_coach_lists_their_own_sessions` |
| SES-28 | `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_coachee_lists_their_own_sessions` |
| SES-29 | `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::only_the_user_lists_their_own_sessions` |
| SES-30 | `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_removed_coachee_lists_none_of_their_own_sessions`; `domain/src/coaching_session_sqlite_tests.rs::a_users_own_sessions_skip_an_org_they_left` |
| SES-31 | by hand: the monthly counts are Postgres-only SQL |
| SES-32 | `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_coachee_reads_the_nearest_session_of_each_relationship`; `domain/src/coaching_session_sqlite_tests.rs::relationships_the_caller_is_not_in_add_nothing`, `domain/src/coaching_session_sqlite_tests.rs::each_relationship_keeps_its_own_window`, `domain/src/coaching_session_sqlite_tests.rs::only_participants_read_the_window` |
| SES-33 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::scheduling_with_a_coachee_who_left_is_refused` |
| SES-34 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::moving_a_session_whose_coachee_left_is_refused` |
| SES-35 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::resending_to_a_coachee_who_left_is_refused` |
| SES-36 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_deletes_a_session_whose_coachee_left` |
| SES-37 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::deleting_a_session_with_content_is_refused` |
| SES-38 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_removed_from_the_org_cannot_create_a_session` |
| SES-39 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::creating_a_session_in_an_unknown_relationship_is_not_found` |
| SES-40 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::an_unknown_session_is_not_found` |
| SES-41 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_removed_from_the_org_cannot_update_a_session` |
| SES-42 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::signed_out_callers_cannot_reach_sessions` |
| SER-01 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::a_coach_creates_a_series_with_its_sessions` |
| SER-02 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::a_coachee_cannot_create_a_series` |
| SER-03 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::non_participants_cannot_create_a_series` |
| SER-04 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::participants_list_the_relationships_series` |
| SER-05 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::non_participants_cannot_list_the_relationships_series` |
| SER-06 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::participants_read_a_series_with_its_sessions` |
| SER-07 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::non_participants_cannot_read_a_series` |
| SER-08 | by hand: the series update takes Postgres advisory locks |
| SER-09 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::only_the_coach_reschedules_a_series` |
| SER-10 | by hand: reschedule takes Postgres advisory locks |
| SER-11 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::only_the_coach_reschedules_following_sessions` |
| SER-12 | by hand: the series delete takes Postgres advisory locks |
| SER-13 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::only_the_coach_deletes_a_series` |
| SER-14 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::a_coach_resends_a_series_invite` |
| SER-15 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::only_the_coach_resends_a_series_invite` |
| SER-16 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::a_coach_removed_from_the_org_cannot_create_a_series` |
| SER-17 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::a_series_with_a_coachee_who_left_is_refused` |
| SER-18 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::rescheduling_a_series_whose_coachee_left_is_refused` |
| SER-19 | by hand: reschedule takes Postgres advisory locks before the membership check |
| SER-20 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::resending_a_series_to_a_coachee_who_left_is_refused` |
| SER-21 | by hand: the series delete takes Postgres advisory locks |
| SER-22 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::an_unknown_series_is_not_found` |
| SER-23 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::signed_out_callers_cannot_reach_series` |
| NOTE-01 | `web/src/controller/note_controller_sqlite_tests.rs::participants_list_the_sessions_notes` |
| NOTE-02 | `web/src/controller/note_controller_sqlite_tests.rs::non_participants_cannot_list_the_sessions_notes` |
| NOTE-03 | `web/src/controller/note_controller_sqlite_tests.rs::a_removed_coachee_cannot_list_their_sessions_notes` |
| NOTE-04 | `web/src/controller/note_controller_sqlite_tests.rs::participants_read_the_note` |
| NOTE-05 | `web/src/controller/note_controller_sqlite_tests.rs::participants_create_a_note_in_the_session` |
| NOTE-06 | `web/src/controller/note_controller_sqlite_tests.rs::participants_update_the_note` |
| NOTE-07 | `web/src/controller/note_controller_sqlite_tests.rs::non_participants_cannot_read_the_note` |
| NOTE-08 | `web/src/controller/note_controller_sqlite_tests.rs::non_participants_cannot_create_a_note_in_the_session` |
| NOTE-09 | `web/src/controller/note_controller_sqlite_tests.rs::non_participants_cannot_update_the_note` |
| NOTE-10 | `web/src/controller/note_controller_sqlite_tests.rs::a_removed_coachee_cannot_reach_their_sessions_notes` |
| AGR-01 | `web/src/controller/agreement_controller_sqlite_tests.rs::participants_list_the_sessions_agreements` |
| AGR-02 | `web/src/controller/agreement_controller_sqlite_tests.rs::non_participants_cannot_list_the_sessions_agreements` |
| AGR-03 | `web/src/controller/agreement_controller_sqlite_tests.rs::a_removed_coachee_cannot_list_their_sessions_agreements` |
| AGR-04 | `web/src/controller/agreement_controller_sqlite_tests.rs::participants_read_the_agreement` |
| AGR-05 | `web/src/controller/agreement_controller_sqlite_tests.rs::participants_create_an_agreement_in_the_session` |
| AGR-06 | `web/src/controller/agreement_controller_sqlite_tests.rs::participants_update_the_agreement` |
| AGR-07 | `web/src/controller/agreement_controller_sqlite_tests.rs::participants_delete_the_agreement` |
| AGR-08 | `web/src/controller/agreement_controller_sqlite_tests.rs::non_participants_cannot_read_the_agreement` |
| AGR-09 | `web/src/controller/agreement_controller_sqlite_tests.rs::non_participants_cannot_create_an_agreement_in_the_session` |
| AGR-10 | `web/src/controller/agreement_controller_sqlite_tests.rs::non_participants_cannot_change_the_agreement` |
| AGR-11 | `web/src/controller/agreement_controller_sqlite_tests.rs::a_removed_coachee_cannot_reach_their_sessions_agreements` |
| ACT-01 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_coach_lists_actions_per_relationship` |
| ACT-02 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_coachee_lists_only_own_and_unassigned_actions_per_relationship` |
| ACT-03 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::another_coach_lists_only_their_relationships_actions` |
| ACT-04 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::members_in_no_relationship_list_no_actions` |
| ACT-05 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::non_members_cannot_list_the_orgs_actions` |
| ACT-06 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_coachee_who_coaches_no_one_cannot_scope_to_the_coach` |
| ACT-07 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_coach_lists_the_relationships_actions` |
| ACT-08 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_coachee_lists_only_own_and_unassigned_relationship_actions` |
| ACT-09 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_coachee_scopes_relationship_actions_only_to_themselves` |
| ACT-10 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::non_participants_cannot_list_the_relationships_actions` |
| ACT-11 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::an_outsider_cannot_list_the_relationships_actions` |
| ACT-12 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_removed_coachee_cannot_list_their_relationships_actions` |
| ACT-13 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_coach_lists_actions_of_a_relationship_whose_coachee_left` |
| ACT-14 | `web/src/controller/action_controller_sqlite_tests.rs::a_coach_lists_the_sessions_actions` |
| ACT-15 | `web/src/controller/action_controller_sqlite_tests.rs::a_coachee_lists_every_action_of_the_session` |
| ACT-16 | `web/src/controller/action_controller_sqlite_tests.rs::non_participants_cannot_list_the_sessions_actions` |
| ACT-17 | `web/src/controller/action_controller_sqlite_tests.rs::a_removed_coachee_cannot_list_their_sessions_actions` |
| ACT-18 | `web/src/controller/user/action_controller_sqlite_tests.rs::a_coachee_lists_every_action_of_their_sessions`; `domain/src/action_sqlite_tests.rs::the_user_sees_every_relationship_they_are_in` |
| ACT-19 | `web/src/controller/user/action_controller_sqlite_tests.rs::a_coachee_lists_the_actions_assigned_to_them`; `domain/src/action_sqlite_tests.rs::the_user_sees_every_assignment_of_theirs` |
| ACT-20 | `web/src/controller/user/action_controller_sqlite_tests.rs::a_coach_lists_their_own_actions` |
| ACT-21 | `web/src/controller/user/action_controller_sqlite_tests.rs::a_removed_coachee_lists_none_of_their_own_actions` |
| ACT-22 | `web/src/controller/user/action_controller_sqlite_tests.rs::a_coach_lists_only_their_relationships_actions_for_the_coachee`; `domain/src/action_sqlite_tests.rs::a_coach_sees_only_their_relationships_with_the_user` |
| ACT-23 | `web/src/controller/user/action_controller_sqlite_tests.rs::a_coach_lists_only_their_relationships_assignments_of_the_coachee`; `domain/src/action_sqlite_tests.rs::a_coach_sees_only_assignments_from_their_relationships` |
| ACT-24 | `web/src/controller/user/action_controller_sqlite_tests.rs::another_coach_lists_only_their_relationships_actions_for_the_coachee`; `domain/src/action_sqlite_tests.rs::another_coach_sees_only_their_own_relationship` |
| ACT-25 | `web/src/controller/user/action_controller_sqlite_tests.rs::members_who_do_not_coach_the_user_cannot_list_their_actions` |
| ACT-26 | `web/src/controller/user/action_controller_sqlite_tests.rs::a_coachee_cannot_list_their_coachs_actions` |
| ACT-26b | `web/src/controller/user/action_controller_sqlite_tests.rs::a_coach_removed_from_the_org_cannot_list_their_coachees_actions` |
| ACT-27 | `web/src/controller/action_controller_sqlite_tests.rs::participants_read_the_action` |
| ACT-28 | `web/src/controller/action_controller_sqlite_tests.rs::participants_create_an_action_assigned_to_the_coachee` |
| ACT-29 | `web/src/controller/action_controller_sqlite_tests.rs::participants_update_the_action` |
| ACT-30 | `web/src/controller/action_controller_sqlite_tests.rs::participants_update_the_action_status` |
| ACT-31 | `web/src/controller/action_controller_sqlite_tests.rs::participants_delete_the_action` |
| ACT-32 | `web/src/controller/action_controller_sqlite_tests.rs::non_participants_cannot_read_the_action` |
| ACT-33 | `web/src/controller/action_controller_sqlite_tests.rs::non_participants_cannot_create_an_action_in_the_session` |
| ACT-34 | `web/src/controller/action_controller_sqlite_tests.rs::non_participants_cannot_change_the_action` |
| ACT-35 | `web/src/controller/action_controller_sqlite_tests.rs::a_removed_coachee_cannot_reach_their_sessions_actions` |
| ACT-36 | `web/src/controller/action_controller_sqlite_tests.rs::assignees_outside_the_relationship_are_refused` |
| ACT-37 | `web/src/controller/action_controller_sqlite_tests.rs::assigning_a_coachee_who_left_is_refused` |
| ACT-38 | `web/src/controller/action_controller_sqlite_tests.rs::a_coach_edits_an_action_still_assigned_to_a_coachee_who_left` |
| ACT-39 | `web/src/controller/action_controller_sqlite_tests.rs::adding_a_coachee_who_left_as_an_assignee_is_refused` |
| ACT-40 | `web/src/controller/action_controller_sqlite_tests.rs::a_goal_from_another_relationship_is_refused` |
| GOAL-01 | `web/src/controller/goal_controller_sqlite_tests.rs::participants_list_the_relationships_goals` |
| GOAL-02 | `web/src/controller/goal_controller_sqlite_tests.rs::non_participants_cannot_list_the_relationships_goals` |
| GOAL-03 | `web/src/controller/goal_controller_sqlite_tests.rs::a_removed_coachee_cannot_list_their_relationships_goals` |
| GOAL-04 | `web/src/controller/goal_controller_sqlite_tests.rs::participants_read_the_goal` |
| GOAL-05 | `web/src/controller/goal_controller_sqlite_tests.rs::non_participants_cannot_read_the_goal` |
| GOAL-06 | `web/src/controller/goal_controller_sqlite_tests.rs::participants_update_the_goal` |
| GOAL-07 | `web/src/controller/goal_controller_sqlite_tests.rs::participants_delete_the_goal` |
| GOAL-08 | `web/src/controller/goal_controller_sqlite_tests.rs::non_participants_cannot_change_the_goal` |
| GOAL-09 | `web/src/controller/goal_controller_sqlite_tests.rs::participants_list_the_goals_sessions` |
| GOAL-10 | `web/src/controller/goal_controller_sqlite_tests.rs::participants_create_a_goal_in_the_relationship` |
| GOAL-11 | `web/src/controller/coaching_session/goal_controller_sqlite_tests.rs::participants_list_the_sessions_goals` |
| GOAL-12 | `web/src/controller/coaching_session/goal_controller_sqlite_tests.rs::non_participants_cannot_list_the_sessions_goals` |
| GOAL-13 | `web/src/controller/coaching_session/goal_controller_sqlite_tests.rs::participants_link_a_goal_to_another_session_of_the_relationship` |
| GOAL-14 | `web/src/controller/coaching_session/goal_controller_sqlite_tests.rs::participants_unlink_a_goal_from_the_session` |
| GOAL-15 | `web/src/controller/coaching_session/goal_controller_sqlite_tests.rs::participants_read_goals_per_session_of_the_relationship` |
| GOAL-16 | `web/src/controller/coaching_session/goal_controller_sqlite_tests.rs::non_participants_cannot_read_goals_per_session_of_the_relationship` |
| GOAL-17 | `web/src/controller/coaching_session/goal_controller_sqlite_tests.rs::every_batch_session_must_be_the_callers` |
| GOAL-18 | `web/src/controller/coaching_session/goal_controller_sqlite_tests.rs::non_participants_cannot_batch_read_the_sessions_goals` |
| GOAL-19 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::participants_read_the_relationships_goal_progress` |
| GOAL-20 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::goal_progress_refuses_a_non_participant_assignee` |
| GOAL-21 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::non_participants_cannot_read_the_relationships_goal_progress` |
| GOAL-22 | `web/src/controller/user/goal_controller_sqlite_tests.rs::a_coach_lists_the_goals_they_created` |
| GOAL-23 | `web/src/controller/user/goal_controller_sqlite_tests.rs::a_coachee_who_created_no_goal_lists_none` |
| GOAL-24 | `web/src/controller/user/goal_controller_sqlite_tests.rs::only_the_user_lists_their_own_goals` |
| GOAL-25 | `web/src/controller/goal_controller_sqlite_tests.rs::non_participants_cannot_create_a_goal_in_the_relationship` |
| GOAL-26 | `web/src/controller/coaching_session/goal_controller_sqlite_tests.rs::non_participants_cannot_change_the_sessions_goal_links` |
| GOAL-27 | `web/src/controller/coaching_session/goal_controller_sqlite_tests.rs::linking_a_goal_from_another_relationship_is_refused` |
| GOAL-28 | `web/src/controller/goal_controller_sqlite_tests.rs::a_created_in_session_from_another_relationship_is_refused` |
| TOP-01 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::participants_list_the_sessions_topics` |
| TOP-02 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::non_participants_cannot_list_the_sessions_topics` |
| TOP-03 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::participants_add_a_topic_authored_by_themselves` |
| TOP-04 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::non_participants_cannot_add_a_topic` |
| TOP-05 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::participants_reorder_the_sessions_topics` |
| TOP-06 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::a_coach_edits_the_coachees_topic` |
| TOP-07 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::a_coachee_edits_the_coachs_topic` |
| TOP-08 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::non_participants_cannot_edit_a_topic` |
| TOP-09 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::a_coach_deletes_the_coachees_topic` |
| TOP-10 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::a_coachee_deletes_their_own_topic` |
| TOP-11 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::a_coachee_cannot_delete_the_coachs_topic` |
| TOP-12 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::non_participants_cannot_delete_a_topic` |
| TOP-16 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::participants_set_a_topics_status` |
| TOP-17 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::non_participants_cannot_set_a_topics_status` |
| TOP-18 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::only_the_author_undoes_a_topics_delete` |
| TOP-19 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::the_author_undoes_a_topics_delete` |
| TOP-20 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::participants_undo_a_topics_defer` |
| TOP-21 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::non_participants_cannot_undo_a_topic_change` |
| TRN-01 | `web/src/controller/coaching_session/transcription_controller_sqlite_tests.rs::participants_read_the_latest_transcription` |
| TRN-02 | `web/src/controller/coaching_session/transcription_controller_sqlite_tests.rs::non_participants_cannot_read_the_latest_transcription` |
| TRN-03 | `web/src/controller/coaching_session/transcription_controller_sqlite_tests.rs::participants_download_only_the_coachs_lines` |
| TRN-04 | `web/src/controller/coaching_session/transcription_controller_sqlite_tests.rs::non_participants_cannot_read_a_transcription` |
| TRN-05 | `web/src/controller/coaching_session/transcription_segment_controller_sqlite_tests.rs::participants_read_the_transcriptions_segments` |
| TRN-06 | `web/src/controller/coaching_session/transcription_segment_controller_sqlite_tests.rs::non_participants_cannot_read_the_transcriptions_segments` |
| TRN-07 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::participants_list_the_sessions_suggestions` |
| TRN-08 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::non_participants_cannot_list_the_sessions_suggestions` |
| TRN-09 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::participants_read_and_start_a_scan` (the POST runs on a fresh unscanned R1 session; S1's finished scan answers 409) |
| TRN-10 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::non_participants_cannot_read_or_start_a_scan` |
| TRN-11 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::a_coach_accepts_a_coach_only_suggestion` |
| TRN-12 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::a_coachee_cannot_accept_a_coach_only_suggestion` |
| TRN-13 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::a_coachee_accepts_their_own_suggestion` |
| TRN-14 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::non_participants_cannot_accept_a_suggestion` |
| TRN-15 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::a_coach_dismisses_and_restores_a_coach_only_suggestion` |
| TRN-16 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::a_coachee_cannot_dismiss_or_restore_a_coach_only_suggestion` |
| TRN-17 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::non_participants_cannot_dismiss_or_restore_a_suggestion` |
| TRN-18 | `web/src/controller/coaching_session/meeting_recording_controller_sqlite_tests.rs::participants_read_the_latest_recording` |
| TRN-19 | by hand: Recall base URL is hard-coded |
| TRN-20 | by hand: stopping the bot needs Recall, whose base URL is hard-coded |
| TRN-21 | `web/src/controller/coaching_session/meeting_recording_controller_sqlite_tests.rs::non_participants_cannot_reach_the_sessions_recording` |
| TRN-22 | `web/src/controller/coaching_session/meeting_recording_controller_sqlite_tests.rs::a_removed_coachee_cannot_reach_their_sessions_recording` |
| TRN-23 | `web/src/controller/coaching_session/image_controller_sqlite_tests.rs::participants_upload_a_note_image` |
| TRN-24 | `web/src/controller/coaching_session/image_controller_sqlite_tests.rs::non_participants_cannot_upload_a_note_image` |
| TRN-25 | `web/src/controller/coaching_session/image_controller_sqlite_tests.rs::participants_read_a_note_image` |
| TRN-26 | `web/src/controller/coaching_session/image_controller_sqlite_tests.rs::either_participant_deletes_and_restores_a_note_image` |
| TRN-27 | `web/src/controller/coaching_session/image_controller_sqlite_tests.rs::non_participants_cannot_read_or_delete_a_note_image` |
| TRN-28 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::accepting_for_a_coachee_who_left_is_refused` |
| MEET-01 | by hand: first-read hydration takes `pg_advisory_xact_lock` and creates the link with the provider |
| MEET-02 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::only_the_coachs_create_attaches_a_meeting_link` (a reused R1 link; a new provider link needs Google or Zoom) |
| MEET-03 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_sets_the_sessions_meeting_link` |
| MEET-04 | by hand: Recall base URL is hard-coded |
| MEET-05 | `web/src/controller/oauth_controller_sqlite_tests.rs::every_caller_reaches_only_their_own_connections` |
| SRCH-01 | by hand: full-text search |
| SRCH-02 | by hand: full-text search |
| SRCH-03 | by hand: full-text search |
| SRCH-04 | `web/src/controller/search_controller_sqlite_tests.rs::members_outside_every_relationship_find_nothing` |
| SRCH-05 | by hand: full-text search |
| SRCH-06 | `web/src/controller/search_controller_sqlite_tests.rs::a_removed_coachee_finds_nothing` |
| SRCH-07 | by hand: full-text search |
| SRCH-08 | `web/src/controller/search_controller_sqlite_tests.rs::a_relationship_filter_never_widens_a_members_search` |
| SRCH-09 | by hand: full-text search |
| SRCH-10 | by hand: full-text search |
| JWT-01 | `web/src/controller/jwt_controller_sqlite_tests.rs::participants_get_a_token_for_the_sessions_document` |
| JWT-02 | `web/src/controller/jwt_controller_sqlite_tests.rs::non_participants_get_no_token` |
| JWT-03 | `web/src/controller/jwt_controller_sqlite_tests.rs::a_removed_coachee_gets_no_token` |
| USR-01 | `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_coachees_sessions_include_their_relationships` |
| USR-02 | `web/src/controller/user/coaching_relationships_controller_sqlite_tests.rs::a_coachee_coaches_no_one` |
| USR-03 | `web/src/controller/user/coaching_relationships_controller_sqlite_tests.rs::a_coach_lists_the_relationships_they_coach` |
| USR-04 | `web/src/controller/user_controller_sqlite_tests.rs::signed_out_callers_cannot_reach_user_routes` |
| USR-05 | `web/src/controller/user_controller_sqlite_tests.rs::only_the_user_changes_their_own_settings` |
| USR-06 | `web/src/controller/user_controller_sqlite_tests.rs::only_admins_look_up_users_by_email` |
| USR-07 | `domain/src/user_role_sqlite_tests.rs::an_admin_finds_a_member_of_their_org` |
| USR-08 | `domain/src/user_role_sqlite_tests.rs::an_admin_cannot_find_a_member_of_another_org_even_with_history_there` |
| USR-09 | `domain/src/user_role_sqlite_tests.rs::an_admin_finds_and_may_administer_a_former_member_of_their_org` |
| USR-10 | `domain/src/user_role_sqlite_tests.rs::a_super_admin_finds_anyone` |
| ORG-01 | `web/src/controller/organization/user_controller_sqlite_tests.rs::admins_add_a_member_with_a_coach` |
| ORG-02 | `web/src/controller/organization/user_controller_sqlite_tests.rs::non_admins_cannot_add_a_member` |
| ORG-03 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::admins_attach_a_member_with_a_coach` |
| ORG-04 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::admins_remove_a_member_and_keep_their_relationship` |
| ORG-05 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::non_admins_cannot_remove_a_members_role` |
| ORG-06 | `web/src/controller/organization/user_controller_sqlite_tests.rs::admins_delete_a_member_without_coaching_history` |
| ORG-07 | `web/src/controller/organization/user_controller_sqlite_tests.rs::an_admin_cannot_delete_themselves` |
| ORG-08 | `web/src/controller/organization/user_controller_sqlite_tests.rs::deleting_a_member_with_coaching_history_is_refused` |
| ORG-09 | `web/src/controller/organization/user_controller_sqlite_tests.rs::non_admins_cannot_resend_a_members_invite` |
| ORG-10 | `web/src/controller/organization/user_controller_sqlite_tests.rs::admins_resend_a_pending_members_invite` |
| ORG-11 | `web/src/controller/organization/user_controller_sqlite_tests.rs::non_admins_cannot_delete_a_member` |
| ORG-12 | `web/src/controller/organization/user_controller_sqlite_tests.rs::an_admin_of_another_org_cannot_manage_its_members` |
| ORG-13 | `web/src/controller/organization/user_controller_sqlite_tests.rs::an_unknown_organization_is_not_found` |
| ORG-14 | `web/src/controller/organization/user_controller_sqlite_tests.rs::a_non_admin_cannot_tell_whether_an_organization_exists` |
| ORG-15 | `web/src/controller/organization/user_controller_sqlite_tests.rs::a_user_outside_the_organization_is_not_found` |
| ORG-16 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::an_admin_cannot_attach_a_user_they_cannot_see` |
| ORG-17 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::an_admin_cannot_tell_a_missing_user_from_one_in_another_org` |
| ORG-18 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::an_admin_of_both_orgs_attaches_a_member_of_the_other` |
| ORG-19 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::attaching_a_current_member_is_a_conflict` |
| ORG-20 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::an_admin_cannot_remove_their_own_role` |
| ORG-21 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::non_admins_cannot_read_or_change_a_role` |
| ORG-22 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::an_admin_cannot_change_their_own_role` |
| ORG-23 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::admins_read_a_members_role` |
| ORG-24 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::admins_change_a_members_role` |
| ORG-25 | `web/src/controller/organization_controller_sqlite_tests.rs::a_super_admin_creates_an_organization` |
| ORG-26 | `web/src/controller/organization_controller_sqlite_tests.rs::only_a_super_admin_creates_an_organization` |
| ORG-27 | `web/src/controller/organization_controller_sqlite_tests.rs::a_super_admin_role_held_in_one_org_cannot_create_an_organization` |
| ORG-28 | `web/src/controller/organization/user_controller_sqlite_tests.rs::signed_out_callers_cannot_reach_organizations` |
| SSE-01 | `web/src/sse/handler_sqlite_tests.rs::every_caller_opens_an_event_stream` |
| SSE-02 | `web/src/sse/handler_sqlite_tests.rs::both_participants_hear_of_an_action_assigned_to_the_coach` |
| SSE-03 | `web/src/sse/handler_sqlite_tests.rs::a_removed_coachee_hears_nothing_of_their_relationship` |
| SSE-04 | `web/src/sse/handler_sqlite_tests.rs::both_participants_hear_of_a_goal_change` |
| SSE-05 | `web/src/sse/handler_sqlite_tests.rs::both_participants_hear_that_suggestions_changed` (through a scan start, which shares the sweep's `extract::notify`); `domain/src/transcript_suggestion/extract_sqlite_tests.rs::every_completed_run_announces_once_with_or_without_suggestions` |
| SSE-06 | `web/src/sse/handler_sqlite_tests.rs::both_participants_hear_of_a_recording_update` |
| JOB-01 | by hand: raw Postgres SQL; SQL shape in `entity_api/src/coaching_session.rs::tests::claim_due_reminders_skips_former_organization_members`, `entity_api/src/coaching_session.rs::tests::claim_due_reminders_membership_mirrors_is_member`, `entity_api/src/coaching_session.rs::tests::claim_due_reminders_skips_sessions_booked_on_short_notice` |
| JOB-02 | `domain/src/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_due_change_is_emailed_and_settled`, `domain/src/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_coachee_no_longer_in_the_organization_is_not_emailed` |
| JOB-03 | `domain/src/transcript_suggestion/prompt_tests.rs::render_writes_the_session_existing_actions_and_transcript`; events as SSE-05 |
| JOB-04 | `domain/src/jobs/coaching_session_image_purge_sqlite_tests.rs` (no recipients) |
| JOB-05 | `domain/src/password_reset.rs::tests::sweep_old_attempts_returns_deleted_count` (no recipients) |
| MAIL-01 | `domain/src/emails_tests.rs::relationship_emails_reach_only_current_members`; `domain/src/emails_sqlite_tests.rs::session_scheduled_with_the_coachee_holding_no_role_reaches_only_the_coach` |
| MAIL-02 | `domain/src/emails_tests.rs::test_send_session_rescheduled_email`; `domain/src/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_coachee_no_longer_in_the_organization_is_not_emailed` |
| MAIL-03 | `domain/src/emails_sqlite_tests.rs::single_cancel_with_the_coachee_holding_no_role_reaches_both`, `domain/src/emails_sqlite_tests.rs::single_cancel_with_the_coach_holding_no_role_reaches_both`, `domain/src/emails_sqlite_tests.rs::a_series_edit_cancels_an_orphan_for_a_coachee_holding_no_role` |
| MAIL-04 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_session_resend_mails_both_members`, `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_who_left_cannot_resend_a_sessions_invite`; `domain/src/emails_sqlite_tests.rs::session_resend_with_the_coach_holding_no_role_reaches_only_the_coachee`; `web/src/controller/resend_invite_tests.rs::session_resend_bumps_a_one_off`, `web/src/controller/resend_invite_tests.rs::resend_with_a_removed_coach_mails_only_the_coachee`, `web/src/controller/resend_invite_tests.rs::resend_to_a_removed_coachee_is_refused` |
| MAIL-05 | `domain/src/emails_tests.rs::relationship_emails_reach_only_current_members`; `domain/src/coaching_session_series.rs::tests::m5_08_series_create_invites_only_current_members` |
| MAIL-06 | `domain/src/emails_tests.rs::relationship_emails_reach_only_current_members`; `domain/src/coaching_session_series.rs::tests::m5_09_series_reschedule_emails_only_current_members` |
| MAIL-07 | `domain/src/emails_tests.rs::relationship_emails_reach_only_current_members`; `domain/src/coaching_session_series.rs::tests::m5_10_reschedule_following_emails_only_current_members` |
| MAIL-08 | `domain/src/emails_sqlite_tests.rs::series_cancel_with_the_coachee_holding_no_role_reaches_both` |
| MAIL-09 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::a_series_resend_mails_both_members`, `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::a_coach_who_left_cannot_resend_a_series_invite`; `domain/src/emails_sqlite_tests.rs::series_resend_with_the_coach_holding_no_role_reaches_only_the_coachee`; `web/src/controller/resend_invite_tests.rs::series_resend_bumps_the_series_and_raises_its_members`, `web/src/controller/resend_invite_tests.rs::resend_with_a_removed_coach_mails_only_the_coachee`, `web/src/controller/resend_invite_tests.rs::resend_to_a_removed_coachee_is_refused` |
| MAIL-10 | `domain/src/emails_tests.rs::test_send_session_reminder_email_goes_only_to_the_coachee` |
| MAIL-11 | `domain/src/emails_tests.rs::action_assigned_reaches_only_current_members` |
| MAIL-12 | `domain/src/emails_tests.rs::test_send_welcome_email_success` |
| MAIL-13 | `domain/src/emails_added_to_organization_tests.rs::send_added_to_organization_email_wire_contract` |
| MAIL-14 | `domain/src/emails_tests.rs::test_send_password_reset_email_wire_contract` |
