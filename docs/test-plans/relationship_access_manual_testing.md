# Test Plan: Relationship Access Matrix

Records who may do what through a coaching relationship today: every route, job and email
that reads or writes relationship-scoped data, per caller, with where the rule is enforced.
It is the regression reference for rs#399: coach and coachee rows must answer the same before
and after that work. Plan: [`colleague-relationships.md`](../implementation-plans/colleague-relationships.md).

Rows record what the code does today, plus the rules tightened by the relationship
authorization security PR (`fix/relationship-scoped-authorization`), which lands before the
rs#399 baseline is frozen. Rows it changes or adds are marked `[tightened]`; their
`Enforced at` cells point at the new checks. Row ids are stable; never renumber them.
Phase 1 adds the `SCHED` rows (4.15): what each schedule change records, with no change to who
may make it. Phase 2 adds the `KIND` rows (4.16): each relationship's kind and the one participant
role type. Phase 4 adds colleague rows, starting with creating one (KIND-04, KIND-11 to KIND-18);
coach and coachee rows answer as before. Phase 5 adds switching a relationship's kind in place
(KIND-28 to KIND-44).

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
  `details.reason` `participant_not_a_member`) when the coachee is no longer a member; assigning an
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

- Agreements and actions, read or written by id or created for a session: the caller
  must be a participant of that session's relationship and still a member of its
  organization (`is_participant_and_member`). Unknown id: 404. Otherwise: 403.
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
| `S4`, `S5` | A future session with no actions or agreements, in R1 and R3. Created only for the delete rows (SES-21 to SES-23, SES-36), so no list row includes them |
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
# A1=... A2=... A3=... A4=... A5=... A6=... A7=... G1=... G2=... AG1=...
# T1=... T2=... IMG1=... TR1=... SG1=... SG2=...
# COACH_ID=... COACHEE_ID=... MEMBER_ID=... REMOVED_ID=... SUPER_ID=...

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
- `is_participant_and_member` (`entity/src/coaching_relationships.rs:125`) is the common participant
  rule: coach or coachee of the relationship AND (global SuperAdmin OR any role in the
  relationship's organization). A non-participant SuperAdmin does not pass it.

## 4. Matrix

### 4.1 Relationships (`REL`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| REL-01 | GET /organizations/O1/coaching_relationships | COACH | 200, R1 R3 | `web/src/extractors/organization_member_access.rs:86`; `domain/src/coaching_relationship.rs:66` | [rows] participant filter |
| REL-02 | GET /organizations/O1/coaching_relationships | COACHEE | 200, R1 R2 | same as REL-01 | [rows] |
| REL-03 | GET /organizations/O1/coaching_relationships | OTHER_COACH | 200, R2 | same as REL-01 | [rows] |
| REL-04 | GET /organizations/O1/coaching_relationships | MEMBER | 200, empty list | same as REL-01 | [rows] |
| REL-05 | GET /organizations/O1/coaching_relationships | ADMIN | 200, R1 R2 R3 | `domain/src/coaching_relationship.rs:52`, `:61`; `entity_api/src/user.rs:236` | [rows] admin sees every relationship in the org |
| REL-06 | GET /organizations/O1/coaching_relationships | SUPER | 200, R1 R2 R3 | `web/src/extractors/organization_member_access.rs:81`; `domain/src/coaching_relationship.rs:61` | [rows] |
| REL-07 | GET /organizations/O1/coaching_relationships | OUTSIDER, REMOVED | 403 | `web/src/extractors/organization_member_access.rs:91` | Not a member of O1 |
| REL-08 | GET /organizations/O1/coaching_relationships/R1 | COACH, COACHEE | 200, R1 with names: `participants[0].first_name` COACH's, `participants[1].first_name` COACHEE's | `web/src/extractors/coaching_relationship_access.rs:86` | |
| REL-09 | GET /organizations/O1/coaching_relationships/R1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/extractors/coaching_relationship_access.rs:86` | ADMIN and SUPER list R1 (REL-05, REL-06) but cannot read it |
| REL-10 | GET /organizations/O2/coaching_relationships/R1 | COACH | 404 | `web/src/extractors/coaching_relationship_access.rs:80` | Path org must match the relationship's |
| REL-11 | GET /organizations/O1/coaching_relationships/R3 | REMOVED | 403 | `web/src/extractors/coaching_relationship_access.rs:86` | Participant, no longer a member |
| REL-12 | GET /organizations/O1/coaching_relationships/R3 | COACH | 200, R3 | `web/src/extractors/coaching_relationship_access.rs:86` | Coach keeps access after the coachee is removed. [membership] `participants[0]` (COACH) `is_member` true, `participants[1]` (REMOVED) `is_member` false |
| REL-13 | GET /users/{COACHEE_ID}/coaching-relationships | COACHEE | 200, R1 R2 | `web/src/extractors/user_self_access.rs:28`; `entity_api/src/coaching_relationship.rs:671` | [rows] self only. `role=coach`: empty; `role=coachee`: R1 R2 |
| REL-14 | GET /users/{COACHEE_ID}/coaching-relationships | COACH, MEMBER, ADMIN, SUPER | 403 | `web/src/extractors/user_self_access.rs:28` | Self only, no admin bypass |
| REL-15 | POST /organizations/O1/coaching_relationships `{coach_id: COACH, coachee_id: MEMBER}` | ADMIN, SUPER | 200 (201), new relationship | `web/src/extractors/organization_admin_access.rs:42`; `entity_api/src/coaching_relationship.rs:74` | Same pair again returns the existing row. Both people must be O1 members |
| REL-16 | POST /organizations/O1/coaching_relationships | COACH, COACHEE, MEMBER, OUTSIDER | 403 | `web/src/extractors/organization_admin_access.rs:48` | |
| REL-17 | GET /users/{REMOVED_ID}/coaching-relationships | REMOVED | 200, empty list | `web/src/extractors/user_self_access.rs:28`; `entity_api/src/coaching_relationship.rs:671`, `:426` | [membership] self list leaves out R3; was 200, R3 |
| REL-18 | GET /organizations/O1/coaching_relationships | ADMIN | 200; R3's `participants` have `is_member` true then false, R1's and R2's both true | `entity_api/src/coaching_relationship.rs:236` | [membership] Every participant carries `is_member`: org list, single read, `GET /users/{id}/coaching-relationships`, the create response (both true), the kind switch response, and session lists' `participants` with `include=relationship` (KIND-49) |
| REL-19 | GET /organizations/{unknown}/coaching_relationships | COACH, ADMIN, SUPER | 404 | `web/src/extractors/organization_member_access.rs:59` | Checked before membership |
| REL-20 | POST /organizations/O1/coaching_relationships | OUTSIDER made an `Admin` of O2 | 403, nothing written | `web/src/extractors/organization_admin_access.rs:48` | An admin of another organization is not an admin of O1 |
| REL-21 | any relationship route | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |
| REL-22 | GET /users/{SUPER_ID}/coaching-relationships | SUPER, once they also coach OUTSIDER in a new O2 relationship R5 | 200, R5 | `web/src/extractors/user_self_access.rs:28`; `web/src/controller/user/coaching_relationships_controller.rs:53`; `domain/src/search.rs:76`; `entity_api/src/coaching_relationship.rs:425` | [rows] [membership] A SuperAdmin's own list is not held to the organizations they hold a role in. SUPER holds no O2 role |
| REL-23 | POST /organizations/O1/coaching_relationships `{coach_id: COACH, coachee_id: MEMBER, kind: "colleague"}` | ADMIN | 200 (201); O1's audit trail ends with `relationship_created` by ADMIN, subject the new relationship, `details` its `kind`, `coach_id`, `coachee_id` | `domain/src/coaching_relationship.rs:137` | Audit checks below the `ORG` table |
| REL-24 | REL-15's create of a colleague pair, then again with the two swapped | ADMIN | both 200 (201), the same relationship; O1's audit trail holds one `relationship_created` | `domain/src/coaching_relationship.rs:133` | A returned relationship is not recorded again |

### 4.2 Sessions (`SES`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| SES-01 | GET /coaching_sessions?coaching_relationship_id=R1&from_date=2000-01-01&to_date=2100-01-01 | COACH, COACHEE | 200, S1 and SR1-* | `web/src/extractors/coaching_relationship_access.rs:139` | [rows] filtered by relationship id. `from_date` and `to_date` are required: 400 without them |
| SES-02 | GET /coaching_sessions?coaching_relationship_id=R1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/extractors/coaching_relationship_access.rs:139` | Unknown relationship id: 404 (`:168`). Missing param: 400 |
| SES-03 | GET /coaching_sessions?coaching_relationship_id=R3 | REMOVED | 403 | `web/src/extractors/coaching_relationship_access.rs:139` | |
| SES-04 | GET /coaching_sessions/S1 | COACH, COACHEE | 200, S1 plus `organization_id` O1 | `web/src/extractors/coaching_session_access.rs:193` | First read hydrates the session, see MEET-01 |
| SES-05 | GET /coaching_sessions/S1 | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | Unknown session: 404 (`:212`) |
| SES-06 | GET /coaching_sessions/S3 | REMOVED | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| SES-07 | POST /coaching_sessions/S1/view | COACH, COACHEE | 200, prior marker | `web/src/extractors/coaching_session_access.rs:193` | Per-caller marker |
| SES-08 | POST /coaching_sessions/S1/view | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| SES-09 | POST /coaching_sessions `{coaching_relationship_id: R1}` | COACH | 200 (201), new session in R1 | `web/src/controller/coaching_session_controller.rs:187`; `web/src/extractors/coaching_relationship_access.rs:155` | [tightened] Coach only, still a member. Sends MAIL-01 |
| SES-10 | POST /coaching_sessions `{coaching_relationship_id: R1}` | COACHEE | 403 | `web/src/controller/coaching_session_controller.rs:187`; `web/src/extractors/coaching_relationship_access.rs:155` | [tightened] Coachee cannot create; colleagues will (can_schedule_sessions, rs#399 phase 3) |
| SES-11 | POST /coaching_sessions `{coaching_relationship_id: R1}` | MEMBER, OTHER_COACH | 403 | `web/src/controller/coaching_session_controller.rs:187`; `web/src/extractors/coaching_relationship_access.rs:155` | [tightened] Unknown relationship: 404 |
| SES-12 | POST /coaching_sessions `{coaching_relationship_id: R1}` | ADMIN, SUPER | 403 | `web/src/controller/coaching_session_controller.rs:187`; `web/src/extractors/coaching_relationship_access.rs:155` | [tightened] |
| SES-13 | POST /coaching_sessions `{coaching_relationship_id: R1}` | OUTSIDER | 403 | `web/src/controller/coaching_session_controller.rs:187`; `web/src/extractors/coaching_relationship_access.rs:155` | [tightened] |
| SES-14 | POST /coaching_sessions `{coaching_relationship_id: R3}` | REMOVED | 403 | `web/src/controller/coaching_session_controller.rs:187`; `web/src/extractors/coaching_relationship_access.rs:155` | [tightened] Not the coach, and no longer a member |
| SES-15 | PUT /coaching_sessions/S1 | COACH | 200 (204) | `web/src/extractors/coaching_session_access.rs:98` | Coach only. A date change queues JOB-02 |
| SES-16 | PUT /coaching_sessions/S1 | COACHEE | 403 | `web/src/extractors/coaching_session_access.rs:98` | |
| SES-17 | PUT /coaching_sessions/S1 | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | Unknown session: 404 |
| SES-18 | PUT /coaching_sessions/S3 | COACH | 200 (204) | `web/src/extractors/coaching_session_access.rs:98` | Coach still a member after the coachee left. [membership] Only when `date` and `duration_minutes` are unchanged, see SES-34 |
| SES-19 | PATCH /coaching_sessions/S1/title | COACH, COACHEE | 200, S1 with new title | `web/src/extractors/coaching_session_access.rs:193` | Either participant |
| SES-20 | PATCH /coaching_sessions/S1/title | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| SES-21 | DELETE /coaching_sessions/S4 | COACH | 200 (204) | `web/src/extractors/coaching_session_access.rs:98` | Sends MAIL-03. A session with content: SES-37 |
| SES-22 | DELETE /coaching_sessions/S4 | COACHEE | 403 | `web/src/extractors/coaching_session_access.rs:98` | |
| SES-23 | DELETE /coaching_sessions/S4 | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| SES-24 | POST /coaching_sessions/S1/resend_invite | COACH | 204 | `web/src/extractors/coaching_session_access.rs:98` | 409 when past or a series member. Sends MAIL-04 |
| SES-25 | POST /coaching_sessions/S1/resend_invite | COACHEE | 403 | `web/src/extractors/coaching_session_access.rs:98` | |
| SES-26 | POST /coaching_sessions/S1/resend_invite | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| SES-27 | GET /users/{COACH_ID}/coaching_sessions | COACH | 200, S1 S3 SR1-* SR3-* | `web/src/extractors/user_self_access.rs:28`; `entity_api/src/coaching_session.rs:1178` | [rows] self only. `include=relationship` adds `relationship` and `participants` |
| SES-28 | GET /users/{COACHEE_ID}/coaching_sessions | COACHEE | 200, S1 S2 SR1-* | same as SES-27 | [rows] |
| SES-29 | GET /users/{COACHEE_ID}/coaching_sessions | COACH, MEMBER, ADMIN, SUPER | 403 | `web/src/extractors/user_self_access.rs:28` | Same gate for `/counts` and `/nearest` |
| SES-30 | GET /users/{REMOVED_ID}/coaching_sessions | REMOVED | 200, empty list | `web/src/extractors/user_self_access.rs:28` | [membership] self list leaves out S3; was 200, S3 |
| SES-31 | GET /users/{COACHEE_ID}/coaching_sessions/counts | COACHEE | 200, counts over S1 S2 SR1-* | `web/src/extractors/user_self_access.rs:28`; `entity_api/src/coaching_session.rs:938` | [rows] |
| SES-32 | GET /users/{COACHEE_ID}/coaching_sessions/nearest?coaching_relationship_ids=R1,R2 | COACHEE | 200, nearest of R1 and of R2 | `web/src/extractors/user_self_access.rs:28`; `entity_api/src/coaching_session.rs:1245` | [rows] a relationship the user is not in contributes nothing |
| SES-33 | POST /coaching_sessions `{coaching_relationship_id: R3}` | COACH | 409 `participant_not_a_member`, nothing written, no email | `domain/src/coaching_session.rs:102` | [membership] `details: {reason, organization_id, user_id}` |
| SES-34 | PUT /coaching_sessions/S3 changing `date` or `duration_minutes` | COACH | 409 `participant_not_a_member`, S3 unchanged | `domain/src/coaching_session.rs:390` | [membership] Other edits succeed (SES-18) |
| SES-35 | POST /coaching_sessions/S3/resend_invite | COACH | 409 `participant_not_a_member`, no email | `domain/src/coaching_session.rs:592` | [membership] |
| SES-36 | DELETE /coaching_sessions/S5 | COACH | 200 (204) | `web/src/extractors/coaching_session_access.rs:98` | [membership] Deleting stays allowed. Sends MAIL-03 to both COACH and REMOVED |
| SES-37 | DELETE /coaching_sessions/S1 | COACH | 409 `sessions_have_content`, `details: {reason, coaching_session_id}`; S1, its content and its notes document untouched, no email | `domain/src/coaching_session.rs:525` | S1 holds AG1 and A1 to A3 |
| SES-38 | POST /coaching_sessions `{coaching_relationship_id: R1}` | COACH after removal from O1 | 403, nothing written, no email | `web/src/controller/coaching_session_controller.rs:187`; `web/src/extractors/coaching_relationship_access.rs:140` | [tightened] Restore the role afterwards |
| SES-39 | POST /coaching_sessions `{coaching_relationship_id: unknown}` | COACH | 404, nothing written, no email | `web/src/extractors/coaching_relationship_access.rs:168` | |
| SES-40 | GET /coaching_sessions/{unknown}, POST /coaching_sessions/{unknown}/resend_invite | COACH | 404, no email | `web/src/extractors/coaching_session_access.rs:212` | |
| SES-41 | PUT /coaching_sessions/S1 | COACH after removal from O1 | 403, S1 unchanged | `web/src/extractors/coaching_session_access.rs:193` | [tightened] Restore the role afterwards |
| SES-42 | any /coaching_sessions route | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |
| SES-43 | GET /users/{REMOVED_ID}/coaching_sessions/nearest?coaching_relationship_ids=R3 | REMOVED | 200, empty list | `web/src/extractors/user_self_access.rs:28`; `entity_api/src/coaching_session.rs:1245`; `entity_api/src/coaching_relationship.rs:426` | [membership] R3's sessions are left out, as in SES-30 |
| SES-44 | GET /users/{SUPER_ID}/coaching_sessions | SUPER, once they also coach OUTSIDER in a new O2 relationship R5 with one session | 200, R5's session | `web/src/extractors/user_self_access.rs:28`; `web/src/controller/user/coaching_session_controller.rs:93`; `domain/src/search.rs:76`; `entity_api/src/coaching_relationship.rs:425` | [rows] [membership] A SuperAdmin's own list is not held to the organizations they hold a role in |
| SES-45 | GET /users/{SUPER_ID}/coaching_sessions/nearest?coaching_relationship_ids=R5 | SUPER, once they also coach OUTSIDER in a new O2 relationship R5 with one session | 200, R5's session | `web/src/extractors/user_self_access.rs:28`; `web/src/controller/user/coaching_session_controller.rs:162`; `domain/src/search.rs:76`; `entity_api/src/coaching_relationship.rs:425` | [rows] [membership] A SuperAdmin's own list is not held to the organizations they hold a role in |
| SES-46 | POST /coaching_sessions `{coaching_relationship_id: RC}`, RC the KIND-04 colleague relationship of COACH and MEMBER, after MEMBER's O1 role is removed (ORG-04) | COACH | 409 `participant_not_a_member`, `details.user_id` MEMBER, nothing written, no email | `domain/src/coaching_relationship.rs:104` | [membership] Either seat's colleague leaving refuses the other. Restore MEMBER's role afterwards |
| SES-47 | POST /coaching_sessions/{RC session}/resend_invite, RC the KIND-04 colleague relationship, a session MEMBER scheduled with a pending reschedule; Resend refusing COACH's copy, then (second run) refusing MEMBER's | MEMBER | first run 502 (network), the pending reschedule kept; second run 204, the pending reschedule cleared | `domain::emails::Delivered::owed`; `domain::coaching_session::ensure_others_told` | A send is done once everyone but whoever scheduled the session is told. Coaching unchanged: the coach scheduled, so the coachee's copy decides |

### 4.3 Series (`SER`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| SER-01 | POST /coaching_session_series `{coaching_relationship_id: R1}` | COACH | 200 (201), series and its sessions | `domain/src/coaching_session_series.rs:94` | Sends MAIL-05 |
| SER-02 | POST /coaching_session_series `{coaching_relationship_id: R1}` | COACHEE | 403 | `domain/src/coaching_session_series.rs:94` | |
| SER-03 | POST /coaching_session_series `{coaching_relationship_id: R1}` | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `domain/src/coaching_session_series.rs:94` | Unknown relationship: 404. [tightened] Coach check now also requires membership, see SER-16 |
| SER-04 | GET /coaching_session_series?coaching_relationship_id=R1 | COACH, COACHEE | 200, SR1 | `web/src/extractors/coaching_relationship_access.rs:139` | [rows] |
| SER-05 | GET /coaching_session_series?coaching_relationship_id=R1 | non-participants | 403 | `web/src/extractors/coaching_relationship_access.rs:139` | |
| SER-06 | GET /coaching_session_series/SR1 | COACH, COACHEE | 200, SR1 and its sessions | `web/src/extractors/coaching_session_series_access.rs:39` | |
| SER-07 | GET /coaching_session_series/SR1 | non-participants | 403 | `web/src/extractors/coaching_session_series_access.rs:39` | Unknown series: 404 |
| SER-08 | PUT /coaching_session_series/SR1 | COACH | 200 | `web/src/extractors/coaching_session_series_access.rs:63` | Sends MAIL-06 |
| SER-09 | PUT /coaching_session_series/SR1 | COACHEE, non-participants | 403 | `web/src/extractors/coaching_session_series_access.rs:63` | |
| SER-10 | POST /coaching_session_series/SR1/reschedule | COACH | 200 | `web/src/extractors/coaching_session_series_access.rs:63` | Sends MAIL-07 |
| SER-11 | POST /coaching_session_series/SR1/reschedule | COACHEE, non-participants | 403 | `web/src/extractors/coaching_session_series_access.rs:63` | |
| SER-12 | DELETE /coaching_session_series/SR1 | COACH | 204 | `web/src/extractors/coaching_session_series_access.rs:63` | Sends MAIL-08 |
| SER-13 | DELETE /coaching_session_series/SR1 | COACHEE, non-participants | 403 | `web/src/extractors/coaching_session_series_access.rs:63` | |
| SER-14 | POST /coaching_session_series/SR1/resend_invite | COACH | 204 | `web/src/extractors/coaching_session_series_access.rs:63` | Sends MAIL-09 |
| SER-15 | POST /coaching_session_series/SR1/resend_invite | COACHEE, non-participants | 403 | `web/src/extractors/coaching_session_series_access.rs:63` | |
| SER-16 | POST /coaching_session_series `{coaching_relationship_id: R1}` | COACH after removal from O1 | 403 | `web/src/controller/coaching_session_series_controller.rs:61`; `web/src/extractors/coaching_relationship_access.rs:155` | [tightened] Run last, or on a disposable relationship: remove the coach from O1 first |
| SER-17 | POST /coaching_session_series `{coaching_relationship_id: R3}` | COACH | 409 `participant_not_a_member`, nothing written, no email | `domain/src/coaching_session_series.rs:103` | [membership] |
| SER-18 | PUT /coaching_session_series/SR3 | COACH | 409 `participant_not_a_member`, SR3 unchanged | `domain/src/coaching_session_series.rs:257` | [membership] |
| SER-19 | POST /coaching_session_series/SR3/reschedule | COACH | 409 `participant_not_a_member`, SR3 unchanged | `domain/src/coaching_session_series.rs:645` | [membership] |
| SER-20 | POST /coaching_session_series/SR3/resend_invite | COACH | 409 `participant_not_a_member`, no email | `domain/src/coaching_session_series.rs:913` | [membership] |
| SER-21 | DELETE /coaching_session_series/SR3 | COACH | 204 | `web/src/extractors/coaching_session_series_access.rs:63` | [membership] Cancelling stays allowed. Sends MAIL-08 to both COACH and REMOVED. Run after SER-18 to SER-20 |
| SER-22 | GET /coaching_session_series/{unknown}; POST its `/resend_invite`, `/reschedule` | COACH | 404, no email | `web/src/extractors/coaching_session_series_access.rs:110` | |
| SER-23 | any /coaching_session_series route | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |
| SER-24 | POST /coaching_session_series `{coaching_relationship_id: RC}`, RC the KIND-04 colleague relationship of COACH and MEMBER, after MEMBER's O1 role is removed (ORG-04) | COACH | 409 `participant_not_a_member`, `details.user_id` MEMBER, nothing written, no email | `domain/src/coaching_relationship.rs:104` | [membership] Restore MEMBER's role afterwards |
| SER-25 | POST /coaching_session_series `{coaching_relationship_id: RC}` with `start_at` before a New York DST change | MEMBER | 200 (201); the series `rule.timezone` is `America/New_York` and the sessions keep New York wall-clock time across the change; the invites' `DTSTART;TZID` is `America/New_York` | `domain/src/coaching_session_series.rs:99` | A colleague series follows whoever created it. Set the zones first: `UPDATE refactor_platform.users SET timezone = 'Asia/Tokyo' WHERE id = '<COACH_ID>'; UPDATE refactor_platform.users SET timezone = 'America/New_York' WHERE id = '<MEMBER_ID>';` |
| SER-26 | PUT /coaching_session_series/SRC, a series on RC that COACH created | MEMBER | 200; `rule.timezone` becomes `America/New_York` | `domain/src/coaching_session_series.rs:212` | A reschedule re-publishes in the zone of whoever made it; reschedule-following (`:624`) the same. Zones as SER-25 |
| SER-27 | POST /coaching_session_series/{SRC}/reschedule_following on a series MEMBER created, with Resend refusing COACH's copy, then (second run) refusing MEMBER's | MEMBER | first run: moved sessions keep their pending reschedules; second run: they settle | `domain::coaching_session_series::record_following_notice` | Each moved session settles once everyone but its scheduler is told |
| SER-28 | PUT /coaching_session_series/SRC, a series on RC that COACH created in Tokyo, with the same start, recurrence and duration | MEMBER (New York) | 200, `changed: false`; session ids unchanged; `rule.timezone` still `Asia/Tokyo`; no email | `domain/src/coaching_session_series.rs` `SeriesRule::schedules_as` | An unchanged save is a no-op whoever saves it. A changed start re-publishes in MEMBER's zone (SER-26). Zones as SER-25 |
| SER-29 | POST /coaching_session_series/{SRC}/reschedule_following on a weekly series on RC that COACH created in New York across March's US and UK DST changes, from a session before both, with that session's own start and no recurrence or duration | MEMBER (London) | 200, `changed: false`; no session's date changes; `rule.timezone` still `America/New_York`; no email | `domain/src/coaching_session_series.rs` `plan_following` (`SeriesRule::schedules_as`) | An unchanged "this and following" save is planned in the series' zone, as SER-28. Set COACH to `America/New_York` and MEMBER to `Europe/London` first |

### 4.4 Agreements (`AGR`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| AGR-01 | GET /agreements?coaching_session_id=S1 | COACH, COACHEE | 200, AG1 | `web/src/extractors/coaching_session_access.rs:193` | [rows] |
| AGR-02 | GET /agreements?coaching_session_id=S1 | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| AGR-03 | GET /agreements?coaching_session_id=S3 | REMOVED | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| AGR-04 | GET /agreements/AG1 | COACH, COACHEE | 200, AG1 | `web/src/extractors/agreement_access.rs:44` | [tightened] |
| AGR-05 | POST /agreements `{coaching_session_id: S1}` | COACH, COACHEE | 200 (201), new agreement | `web/src/controller/agreement_controller.rs:51` | [tightened] |
| AGR-06 | PUT /agreements/AG1 | COACH, COACHEE | 200, AG1 updated | `web/src/extractors/agreement_access.rs:44` | [tightened] |
| AGR-07 | DELETE /agreements/AG1 | COACH, COACHEE | 200, `{id}` | `web/src/extractors/agreement_access.rs:44` | [tightened] Run last in this table |
| AGR-08 | GET /agreements/AG1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/extractors/agreement_access.rs:44` | [tightened] Unknown agreement: 404 |
| AGR-09 | POST /agreements `{coaching_session_id: S1}` | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, nothing written | `web/src/controller/agreement_controller.rs:51` | [tightened] Unknown session: 404 |
| AGR-10 | PUT, DELETE /agreements/AG1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, AG1 unchanged | `web/src/extractors/agreement_access.rs:44` | [tightened] |
| AGR-11 | GET, PUT, DELETE an agreement in S3; POST `{coaching_session_id: S3}` | REMOVED | 403 | `web/src/extractors/agreement_access.rs:44`; `web/src/controller/agreement_controller.rs:51` | [tightened] |
| AGR-12 | any /agreements route | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |

### 4.5 Actions (`ACT`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| ACT-01 | GET /organizations/O1/coaching_relationships/actions | COACH | 200, `actions_by_relationship`: R1 [A1 A2 A3], R3 [A6 A7]; no `coachee_actions` | `web/src/extractors/organization_member_access.rs:86`; `entity_api/src/action.rs:615` | [rows] keyed by relationship id |
| ACT-02 | GET /organizations/O1/coaching_relationships/actions | COACHEE | 200, R1 [A2 A3], R2 [A5]; not A1, A4 | `entity_api/src/action.rs:482`, `:475` | [rows] coachee side sees own or unassigned |
| ACT-03 | GET /organizations/O1/coaching_relationships/actions | OTHER_COACH | 200, R2 [A4 A5] | `entity_api/src/action.rs:615` | [rows] |
| ACT-04 | GET /organizations/O1/coaching_relationships/actions | MEMBER, ADMIN, SUPER | 200, empty object | `web/src/controller/organization/coaching_relationship/actions_controller.rs:158` | [rows] only the caller's own relationships |
| ACT-05 | GET /organizations/O1/coaching_relationships/actions | OUTSIDER, REMOVED | 403 | `web/src/extractors/organization_member_access.rs:91` | |
| ACT-06 | GET /organizations/O1/coaching_relationships/actions?assignee=coach | COACHEE | 403 | `web/src/controller/organization/coaching_relationship/actions_controller.rs:169`, `:45` | Only when the caller coaches no relationship |
| ACT-07 | GET /organizations/O1/coaching_relationships/R1/actions | COACH | 200, A1 A2 A3 | `web/src/extractors/coaching_relationship_access.rs:86`; `web/src/controller/organization/coaching_relationship/actions_controller.rs:98` | [rows] |
| ACT-08 | GET /organizations/O1/coaching_relationships/R1/actions | COACHEE | 200, A2 A3; not A1 | `web/src/controller/organization/coaching_relationship/actions_controller.rs:98`; `entity_api/src/action.rs:578` | [rows] |
| ACT-09 | GET /organizations/O1/coaching_relationships/R1/actions?assignee=coach | COACHEE | 403 | `web/src/controller/organization/coaching_relationship/actions_controller.rs:100` | `assignee=coachee` or own id: 200, A2 |
| ACT-10 | GET /organizations/O1/coaching_relationships/R1/actions | MEMBER, OTHER_COACH, ADMIN, SUPER | 403 | `web/src/extractors/coaching_relationship_access.rs:86` | |
| ACT-11 | GET /organizations/O1/coaching_relationships/R1/actions | OUTSIDER | 403 | `web/src/extractors/organization_member_access.rs:91` | Member check runs first |
| ACT-12 | GET /organizations/O1/coaching_relationships/R3/actions | REMOVED | 403 | `web/src/extractors/organization_member_access.rs:91` | |
| ACT-13 | GET /organizations/O1/coaching_relationships/R3/actions | COACH | 200, A6 A7 | `web/src/extractors/coaching_relationship_access.rs:86` | [rows] |
| ACT-14 | GET /actions?coaching_session_id=S1 | COACH | 200, A1 A2 A3 | `web/src/extractors/coaching_session_access.rs:193` | [rows] |
| ACT-15 | GET /actions?coaching_session_id=S1 | COACHEE | 200, A1 A2 A3 | `web/src/extractors/coaching_session_access.rs:193`; `web/src/controller/action_controller.rs:361` | [rows] no assignee narrowing here |
| ACT-16 | GET /actions?coaching_session_id=S1 | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| ACT-17 | GET /actions?coaching_session_id=S3 | REMOVED | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| ACT-18 | GET /users/{COACHEE_ID}/actions (scope=sessions, default) | COACHEE | 200, A1 A2 A3 A4 A5 | `web/src/extractors/user_actions_access.rs:36`; `entity_api/src/action.rs:391` | [rows] self access; no assignee narrowing |
| ACT-19 | GET /users/{COACHEE_ID}/actions?scope=assigned | COACHEE | 200, A2 A5 | `web/src/extractors/user_actions_access.rs:36`; `entity_api/src/action.rs:349` | [rows] |
| ACT-20 | GET /users/{COACH_ID}/actions (scope=sessions) | COACH | 200, A1 A2 A3 A6 A7 | `web/src/extractors/user_actions_access.rs:36`; `entity_api/src/action.rs:391` | [rows] `scope=assigned`: A1 |
| ACT-21 | GET /users/{REMOVED_ID}/actions (scope=sessions) | REMOVED | 200, empty list | `web/src/extractors/user_actions_access.rs:36`; `entity_api/src/action.rs:391` | [membership] self list leaves out A6; was 200, A6 |
| ACT-22 | GET /users/{COACHEE_ID}/actions (scope=sessions) | COACH | 200, A1 A2 A3; not A4 A5 | `web/src/extractors/user_actions_access.rs:40`; `web/src/controller/user/action_controller.rs:86`; `entity_api/src/action.rs:393`; `entity_api/src/coaching_relationship.rs:416` | [rows] [tightened] only relationships where the caller coaches this user |
| ACT-23 | GET /users/{COACHEE_ID}/actions?scope=assigned | COACH | 200, A2; not A5 | `web/src/extractors/user_actions_access.rs:40`; `web/src/controller/user/action_controller.rs:86`; `entity_api/src/action.rs:393`; `entity_api/src/coaching_relationship.rs:416` | [rows] [tightened] |
| ACT-24 | GET /users/{COACHEE_ID}/actions (scope=sessions) | OTHER_COACH | 200, A4 A5; not A1 A2 A3 | `web/src/extractors/user_actions_access.rs:40`; `web/src/controller/user/action_controller.rs:86`; `entity_api/src/action.rs:393`; `entity_api/src/coaching_relationship.rs:416` | [rows] [tightened] |
| ACT-25 | GET /users/{COACHEE_ID}/actions | MEMBER, ADMIN, SUPER | 403 | `web/src/extractors/user_actions_access.rs:40`; `web/src/controller/user/action_controller.rs:86`; `entity_api/src/action.rs:393`; `entity_api/src/coaching_relationship.rs:416` | [tightened] |
| ACT-26 | GET /users/{COACH_ID}/actions | COACHEE | 403 | `web/src/extractors/user_actions_access.rs:40`; `web/src/controller/user/action_controller.rs:86`; `entity_api/src/action.rs:393`; `entity_api/src/coaching_relationship.rs:416` | [tightened] A coachee is not the coach's coach |
| ACT-26b | GET /users/{COACHEE_ID}/actions | COACH after their O1 role is removed | 403 | `web/src/extractors/user_actions_access.rs:40`; `entity_api/src/coaching_relationship.rs:420` | [tightened] A coach must still be a member of the relationship's org; a coach still in another org sees only that org's relationships. Restore the role afterwards |
| ACT-27 | GET /actions/A1 | COACH, COACHEE | 200, A1 | `web/src/extractors/action_access.rs:46` | [tightened] |
| ACT-28 | POST /actions `{coaching_session_id: S1, assignee_ids: [COACHEE_ID]}` | COACH, COACHEE | 200 (201), new action | `web/src/controller/action_controller.rs:120` | [tightened] Sends MAIL-11 |
| ACT-29 | PUT /actions/A1 | COACH, COACHEE | 200, A1 updated | `web/src/extractors/action_access.rs:46` | [tightened] Newly added assignees get MAIL-11 |
| ACT-30 | PUT /actions/A1/status?value=on_hold | COACH, COACHEE | 200, A1 | `web/src/extractors/action_access.rs:46` | [tightened] |
| ACT-31 | DELETE /actions/A1 | COACH, COACHEE | 200, `{id}` | `web/src/extractors/action_access.rs:46` | [tightened] Run last in this table |
| ACT-32 | GET /actions/A1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403 | `web/src/extractors/action_access.rs:46` | [tightened] Unknown action: 404 |
| ACT-33 | POST /actions `{coaching_session_id: S1}` | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, nothing written, no email | `web/src/controller/action_controller.rs:120` | [tightened] Unknown session: 404 |
| ACT-34 | PUT /actions/A1, PUT /actions/A1/status, DELETE /actions/A1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, A1 unchanged | `web/src/extractors/action_access.rs:46` | [tightened] |
| ACT-35 | GET, PUT, DELETE A6; POST `{coaching_session_id: S3}` | REMOVED | 403 | `web/src/extractors/action_access.rs:46`; `web/src/controller/action_controller.rs:120` | [tightened] |
| ACT-36 | POST /actions `{coaching_session_id: S1, assignee_ids: [MEMBER_ID]}`; PUT /actions/A3 adding MEMBER_ID | COACH | 422, nothing written, no email; `message` `assignee_ids must name participants in the action's relationship.` | `web/src/controller/action_controller.rs:63` (via `validate_links`, `:124`, `:249`) | [tightened] Assignees must be the relationship's participants |
| ACT-37 | POST /actions `{coaching_session_id: S3, assignee_ids: [REMOVED_ID]}` | COACH | 409 `assignee_not_a_member`, nothing written, no email | `web/src/controller/action_controller.rs:77`; `domain/src/coaching_relationship.rs:105` | [membership] `details: {reason, organization_id, user_id}` |
| ACT-38 | PUT /actions/A6 editing the body, REMOVED still assigned | COACH | 200, A6 updated, no email | `web/src/controller/action_controller.rs:77` | [membership] Only newly added assignees are checked |
| ACT-39 | PUT /actions/A7 adding REMOVED_ID as an assignee | COACH | 409 `assignee_not_a_member`, A7 unchanged, no email | `web/src/controller/action_controller.rs:77`; `domain/src/coaching_relationship.rs:117` | [membership] `details: {reason, organization_id, user_id}` |
| ACT-40 | POST /actions `{coaching_session_id: S1, goal_id: G2}` | COACH | 422, nothing written | `web/src/controller/action_controller.rs:83` | [tightened] G2 belongs to R2; an action's goal must be in its relationship |
| ACT-41 | GET /organizations/O1/coaching_relationships/actions | COACHEE, once they also coach MEMBER in a new O1 relationship R4 whose session holds one action each for COACHEE, MEMBER and nobody | 200, R1 [A2 A3], R2 [A5], R4 [all three] | `entity_api/src/action.rs:615`, `:497` | [rows] Visibility is decided per relationship: everything where the caller coaches, own or unassigned where they are the coachee |
| ACT-42 | any actions route (`/actions`, `/users/{id}/actions`, the organization action lists) | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |
| ACT-43 | GET /users/{SUPER_ID}/actions (scope=sessions) | SUPER, once they also coach OUTSIDER in a new O2 relationship R5 whose session holds one action each for SUPER, OUTSIDER and nobody | 200, all three | `web/src/extractors/user_actions_access.rs:36`; `web/src/controller/user/action_controller.rs:88`; `domain/src/search.rs:76`; `entity_api/src/coaching_relationship.rs:425` | [rows] [membership] A SuperAdmin's own list is not held to the organizations they hold a role in. `scope=assigned`: SUPER's action |
| ACT-44 | GET /organizations/O1/coaching_relationships/RC/actions?assignee=coach, and `?assignee=coachee`, RC the KIND-04 colleague relationship of COACH and MEMBER | COACH, MEMBER | 422, `details.reason` `assignee_scope_not_applicable` | `web/src/controller/organization/coaching_relationship/actions_controller.rs:134`; `domain/src/action.rs:24` | Colleagues hold no coach or coachee seat to scope to |
| ACT-45 | GET /organizations/O1/coaching_relationships/RC/actions, and `?assignee={MEMBER_ID}` | COACH, MEMBER | 200, every RC action; with the id, only MEMBER's | `entity_api/src/action.rs:470` | [rows] Both colleagues see every action |
| ACT-46 | GET /organizations/O1/coaching_relationships/actions?assignee=coachee, COACH coaching R1 and R3 and a colleague of MEMBER in RC | COACH | 200, R1 [A2], R3 [A6]; no RC key. Without `assignee`: R1, R3 and RC in full | `entity_api/src/action.rs:646` | [rows] The batch leaves colleague relationships out of a seat scope and never refuses for them |
| ACT-47 | GET /organizations/O1/coaching_relationships/actions?assignee=coach (and `coachee`), MEMBER in RC and no other relationship | MEMBER | 200, empty object | `web/src/controller/organization/coaching_relationship/actions_controller.rs:36` | No 403: a colleague sees every action |
| ACT-48 | GET /organizations/O1/coaching_relationships/actions?assignee=coach, COACHEE also a colleague of MEMBER in a new O1 relationship | COACHEE | 403 | `web/src/controller/organization/coaching_relationship/actions_controller.rs:36`, `:191` | ACT-06 still holds: the 403 is decided over the relationships the scope applies to |
| ACT-49 | GET /organizations/O1/coaching_relationships/actions?assignee=colleague, COACH coaching R1 and R3 and a colleague of MEMBER in RC, RC's session holding one action each for COACH, MEMBER, nobody, and both COACH and MEMBER | COACH | 200, RC [MEMBER's, the shared one]; no R1 or R3 key | `entity_api/src/action.rs:474`, `:654` | [rows] `colleague` names the participant holding that role who isn't the caller; coaching relationships hold no colleague and are left out |
| ACT-50 | GET /organizations/O1/coaching_relationships/actions?assignee=colleague, the caller in coaching relationships only | COACH, COACHEE, OTHER_COACH | 200, empty object | `web/src/controller/organization/coaching_relationship/actions_controller.rs:36`; `entity_api/src/action.rs:654` | No 403, for a coachee too |
| ACT-51 | GET /organizations/O1/coaching_relationships/R1/actions?assignee=colleague with R1 made a colleague relationship and one more action assigned to both; then R2 (coaching) | COACH, COACHEE on R1; OTHER_COACH, COACHEE on R2 | R1: 200, COACH gets [A2, the shared one], COACHEE gets [A1, the shared one]. R2: 422, `details.reason` `assignee_scope_not_applicable` | `web/src/controller/organization/coaching_relationship/actions_controller.rs:71`, `:137`; `domain/src/action.rs:24` | [rows] Each colleague gets the other's actions. goal_progress takes the same rule |

### 4.6 Goals (`GOAL`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| GOAL-01 | GET /goals?coaching_relationship_id=R1 | COACH, COACHEE | 200, G1 | `web/src/extractors/coaching_relationship_access.rs:139` | [rows] |
| GOAL-02 | GET /goals?coaching_relationship_id=R1 | non-participants | 403 | `web/src/extractors/coaching_relationship_access.rs:139` | |
| GOAL-03 | GET /goals?coaching_relationship_id=R3 | REMOVED | 403 | `web/src/extractors/coaching_relationship_access.rs:139` | |
| GOAL-04 | GET /goals/G1 | COACH, COACHEE | 200, G1 | `web/src/extractors/goal_access.rs:45` | Same gate: PUT, DELETE, PUT `/status`, GET `/sessions`, GET `/progress` |
| GOAL-05 | GET /goals/G1 | non-participants | 403 | `web/src/extractors/goal_access.rs:45` | Unknown goal: 404 |
| GOAL-06 | PUT /goals/G1 | COACH, COACHEE | 200, G1 updated | `web/src/extractors/goal_access.rs:45` | |
| GOAL-07 | DELETE /goals/G1 | COACH, COACHEE | 200, `{id}` | `web/src/extractors/goal_access.rs:45` | Either participant |
| GOAL-08 | PUT, DELETE /goals/G1 | non-participants | 403 | `web/src/extractors/goal_access.rs:45` | |
| GOAL-09 | GET /goals/G1/sessions | COACH, COACHEE | 200, S1 | `web/src/extractors/goal_access.rs:45` | [rows] |
| GOAL-10 | POST /goals `{coaching_relationship_id: R1}` | COACH, COACHEE | 200 (201), new goal | `web/src/controller/goal_controller.rs:85` | [tightened] Non-participants: GOAL-25 |
| GOAL-11 | GET /coaching_sessions/S1/goals | COACH, COACHEE | 200, G1 | `web/src/extractors/coaching_session_access.rs:193` | [rows] |
| GOAL-12 | GET /coaching_sessions/S1/goals | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| GOAL-13 | POST /coaching_sessions/SR1-1/goals `{goal_id: G1}` (an R1 session G1 isn't linked to yet) | COACH, COACHEE | 200 (201), link | `web/src/controller/coaching_session/goal_controller.rs:48` | [tightened] Non-participants: GOAL-26 |
| GOAL-14 | DELETE /coaching_sessions/S1/goals/G1 | COACH, COACHEE | 200, `{goal_id}` | `web/src/controller/coaching_session/goal_controller.rs:98` | [tightened] Non-participants: GOAL-26 |
| GOAL-15 | GET /coaching_sessions/goals?coaching_relationship_id=R1 | COACH, COACHEE | 200, `session_goals` S1 [G1], SR1-* [] | `web/src/extractors/coaching_session_goal_access.rs:45` | [rows] |
| GOAL-16 | GET /coaching_sessions/goals?coaching_relationship_id=R1 | non-participants | 403 | `web/src/extractors/coaching_session_goal_access.rs:45` | |
| GOAL-17 | GET /coaching_sessions/goals?coaching_session_ids=S1,S2 | COACH | 403 | `web/src/extractors/coaching_session_goal_access.rs:47` | [tightened] Every id is authorized; S2 is in R2. `S1` alone: 200, S1 [G1] |
| GOAL-18 | GET /coaching_sessions/goals?coaching_session_ids=S1 | non-participants | 403 | `web/src/extractors/coaching_session_goal_access.rs:76` | |
| GOAL-19 | GET /organizations/O1/coaching_relationships/R1/goal_progress | COACH, COACHEE | 200, progress for G1 | `web/src/extractors/coaching_relationship_access.rs:86` | [rows] `assignee=coach`/`coachee`/participant id allowed |
| GOAL-20 | GET /organizations/O1/coaching_relationships/R1/goal_progress?assignee={MEMBER_ID} | COACH | 400 | `web/src/controller/organization/coaching_relationship_controller.rs:214` | Non-participant id rejected |
| GOAL-21 | GET /organizations/O1/coaching_relationships/R1/goal_progress | non-participants | 403 | `web/src/extractors/coaching_relationship_access.rs:86` | No separate member extractor on this route |
| GOAL-22 | GET /users/{COACH_ID}/goals | COACH | 200, G1 | `web/src/extractors/user_self_access.rs:28` | [rows] goals the user created |
| GOAL-23 | GET /users/{COACHEE_ID}/goals | COACHEE | 200, empty list | `web/src/extractors/user_self_access.rs:28` | [rows] COACHEE created no goal |
| GOAL-24 | GET /users/{COACHEE_ID}/goals | COACH, MEMBER, ADMIN, SUPER | 403 | `web/src/extractors/user_self_access.rs:28` | Self only |
| GOAL-25 | POST /goals `{coaching_relationship_id: R1}` | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, nothing written | `web/src/controller/goal_controller.rs:85` | [tightened] Unknown relationship: 404. REMOVED on R3: 403 |
| GOAL-26 | POST /coaching_sessions/S1/goals, DELETE /coaching_sessions/S1/goals/G1 | MEMBER, OTHER_COACH, ADMIN, SUPER, OUTSIDER | 403, links unchanged | `web/src/controller/coaching_session/goal_controller.rs:48`, `:98` | [tightened] REMOVED on S3: 403 |
| GOAL-27 | POST /coaching_sessions/S1/goals `{goal_id: G2}` | COACH | 422, nothing linked | `web/src/controller/coaching_session/goal_controller.rs:60`, `:59` | [tightened] G2 belongs to R2; a goal links only to sessions in its own relationship. Unknown goal: 404 |
| GOAL-28 | POST /goals `{coaching_relationship_id: R1, created_in_session_id: S2}` | COACH | 422, nothing written | `web/src/controller/goal_controller.rs:93` | [tightened] S2 is in R2; a new goal's created-in session must be in its relationship. Unknown session: 404 |
| GOAL-29 | PUT /goals/G1/status?value=on_hold | COACH, COACHEE | 200, G1 with status `OnHold` | `web/src/extractors/goal_access.rs:45` | |
| GOAL-30 | GET /goals/G1/progress | COACH, COACHEE | 200, G1's progress: one linked session, no actions | `web/src/extractors/goal_access.rs:45` | |
| GOAL-31 | PUT /goals/G1/status, GET /goals/G1/progress | non-participants | 403, G1 unchanged | `web/src/extractors/goal_access.rs:45` | Unknown goal: 404 |
| GOAL-32 | POST /goals `{coaching_relationship_id: R1, created_in_session_id: S1}` | COACH | 200 (201), new goal linked to S1 | `web/src/controller/goal_controller.rs:93` | The allowed side of GOAL-28 |
| GOAL-33 | any goal route (`/goals`, `/coaching_sessions/.../goals`, `/users/{id}/goals`, `goal_progress`) | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |
| GOAL-34 | GET /users/{REMOVED_ID}/goals, REMOVED having created a goal in R3 | REMOVED | 200, empty list | `web/src/extractors/user_self_access.rs:28`; `entity_api/src/goal.rs:38`; `entity_api/src/coaching_relationship.rs:426` | [membership] self list leaves out goals in R3 |
| GOAL-35 | GET /users/{SUPER_ID}/goals | SUPER, once they also coach OUTSIDER in a new O2 relationship R5, SUPER having created a goal in R5 | 200, that goal | `web/src/extractors/user_self_access.rs:28`; `web/src/controller/user/goal_controller.rs:56`; `domain/src/search.rs:76`; `entity_api/src/coaching_relationship.rs:425` | [rows] [membership] A SuperAdmin's own list is not held to the organizations they hold a role in |
| GOAL-36 | GET /organizations/O1/coaching_relationships/RC/goal_progress?assignee=coach (and `coachee`), RC the KIND-04 colleague relationship | COACH | 422, `details.reason` `assignee_scope_not_applicable`; `?assignee={MEMBER_ID}`: 200 | `web/src/controller/organization/coaching_relationship_controller.rs:219`; `domain/src/action.rs:24` | Same rule as ACT-44 |

### 4.7 Topics (`TOP`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| TOP-01 | GET /coaching_sessions/S1/topics | COACH, COACHEE | 200, T1 T2 | `web/src/extractors/coaching_session_access.rs:193` | [rows] |
| TOP-02 | GET /coaching_sessions/S1/topics | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| TOP-03 | POST /coaching_sessions/S1/topics | COACH, COACHEE | 200 (201), new topic authored by caller | `web/src/extractors/coaching_session_access.rs:193` | |
| TOP-04 | POST /coaching_sessions/S1/topics | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| TOP-05 | PATCH /coaching_sessions/S1/topics/reorder | COACH, COACHEE | 200, T1 T2 reordered | `web/src/extractors/coaching_session_access.rs:193` | |
| TOP-06 | PUT /coaching_sessions/S1/topics/T2 | COACH | 200, T2 | `web/src/extractors/coaching_session_topic_access.rs:41`, `:47` | Any participant edits any topic |
| TOP-07 | PUT /coaching_sessions/S1/topics/T1 | COACHEE | 200, T1 | `web/src/extractors/coaching_session_topic_access.rs:41`, `:47` | |
| TOP-08 | PUT /coaching_sessions/S1/topics/T1 | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:41` | Topic not in S1: 404 |
| TOP-09 | DELETE /coaching_sessions/S1/topics/T2 | COACH | 200 | `web/src/extractors/coaching_session_topic_access.rs:94` | Coach deletes any topic |
| TOP-10 | DELETE /coaching_sessions/S1/topics/T2 | COACHEE | 200 | `web/src/extractors/coaching_session_topic_access.rs:82` | Author |
| TOP-11 | DELETE /coaching_sessions/S1/topics/T1 | COACHEE | 404 | `web/src/extractors/coaching_session_topic_access.rs:96` | Not author, not coach |
| TOP-12 | DELETE /coaching_sessions/S1/topics/T1 | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:76`, `:48` | Topic not in S1: 404 |
| TOP-16 | PATCH /coaching_sessions/S1/topics/T1/status | COACH, COACHEE | 200, T1 | `web/src/extractors/coaching_session_topic_access.rs:41`, `:47` | Either participant |
| TOP-17 | PATCH /coaching_sessions/S1/topics/T1/status | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:41` | |
| TOP-18 | POST /coaching_sessions/S1/topics/T2/undo (T2 deleted by COACH) | COACH | 404 | `web/src/extractors/coaching_session_topic_access.rs:139` | Undoing a delete is author only |
| TOP-19 | POST /coaching_sessions/S1/topics/T2/undo (T2 deleted) | COACHEE | 200, T2 restored | `web/src/extractors/coaching_session_topic_access.rs:139` | |
| TOP-20 | POST /coaching_sessions/{dest}/topics/T1/undo (T1 deferred) | COACH, COACHEE | 200, T1 back in S1 | `web/src/extractors/coaching_session_topic_access.rs:136` | Defer undo open to either participant |
| TOP-21 | POST /coaching_sessions/S1/topics/T1/undo | non-participants | 403 | `web/src/extractors/coaching_session_topic_access.rs:122`, `:131` | Topic not in S1: 404 |
| TOP-22 | any topic route | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |

### 4.8 Transcripts, recordings and images (`TRN`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| TRN-01 | GET /coaching_sessions/S1/transcriptions | COACH, COACHEE | 200, TR1 with `speakers` | `web/src/extractors/coaching_session_access.rs:193` | |
| TRN-02 | GET /coaching_sessions/S1/transcriptions | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| TRN-03 | GET /coaching_sessions/S1/transcriptions/TR1 (`Accept: text/plain`, `speaker=coach`) | COACH, COACHEE | 200, coach lines only | `web/src/extractors/coaching_session_access.rs:193`; `domain/src/transcription.rs:579` | 422 if no speaker attributed to the role |
| TRN-04 | GET /coaching_sessions/S1/transcriptions/TR1 | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193`; `domain/src/transcription.rs:554` | TR1 under another session: 404 `transcription_not_found` |
| TRN-05 | GET /coaching_sessions/S1/transcriptions/TR1/transcription_segments | COACH, COACHEE | 200, TR1 segments | `web/src/extractors/coaching_session_access.rs:193` | |
| TRN-06 | GET /coaching_sessions/S1/transcriptions/TR1/transcription_segments | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193`; `entity_api/src/transcript_segment.rs:41` | TR1 under another session: 200, no segments |
| TRN-07 | GET /coaching_sessions/S1/transcript_suggestions | COACH, COACHEE | 200, SG1 SG2 | `web/src/extractors/coaching_session_access.rs:193`; `domain/src/transcript_suggestion/review.rs:247` | [rows] coachee also sees coach-only SG1 |
| TRN-08 | GET /coaching_sessions/S1/transcript_suggestions | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| TRN-09 | GET, POST /coaching_sessions/S1/transcript_suggestions/scan | COACH, COACHEE | GET 200, POST 202; scan status | `web/src/extractors/coaching_session_access.rs:193` | POST: 409 if running or done |
| TRN-10 | GET, POST /coaching_sessions/S1/transcript_suggestions/scan | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| TRN-11 | POST /coaching_sessions/S1/transcript_suggestions/SG1/accept | COACH | 200 (201), new action | `domain/src/transcript_suggestion/review.rs:333`, `:553`, `:202` | Coach may always act. Sends MAIL-11 |
| TRN-12 | POST /coaching_sessions/S1/transcript_suggestions/SG1/accept | COACHEE | 403 | `domain/src/transcript_suggestion/review.rs:206` | Coach-only suggestion |
| TRN-13 | POST /coaching_sessions/S1/transcript_suggestions/SG2/accept | COACHEE | 200 (201), new action | `domain/src/transcript_suggestion/review.rs:206` | Edits that add or remove the coach: 403 |
| TRN-14 | POST /coaching_sessions/S1/transcript_suggestions/SG1/accept | non-participants | 404 | `web/src/extractors/transcript_suggestion_access.rs:45`, `:58` | Refusals collapse to 404. SG1 under another session: 404 |
| TRN-15 | POST .../SG1/dismiss, .../SG1/restore | COACH | 204 | `domain/src/transcript_suggestion/review.rs:438`, `:553` | |
| TRN-16 | POST .../SG1/dismiss, .../SG1/restore | COACHEE | 403 | `domain/src/transcript_suggestion/review.rs:206` | SG2: 204 |
| TRN-17 | POST .../SG1/dismiss, .../SG1/restore | non-participants | 404 | `web/src/extractors/transcript_suggestion_access.rs:45`, `:55`, `:58` | SG1 under another session, or an unknown suggestion: 404 |
| TRN-18 | GET /coaching_sessions/S1/meeting_recording | COACH, COACHEE | 200, latest recording or null | `web/src/extractors/coaching_session_access.rs:193` | |
| TRN-19 | POST /coaching_sessions/S1/meeting_recording `{meeting_url}` | COACH, COACHEE | 200 (201), recording | `web/src/extractors/coaching_session_access.rs:193` | Either participant starts a bot. The first start wins; a concurrent second start is a 409 (`details.reason` `recording_already_active`) and creates no bot |
| TRN-20 | DELETE /coaching_sessions/S1/meeting_recording | COACH, COACHEE | 200 | `web/src/extractors/coaching_session_access.rs:193` | |
| TRN-21 | GET, POST, DELETE /coaching_sessions/S1/meeting_recording | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| TRN-22 | GET, POST, DELETE /coaching_sessions/S3/meeting_recording | REMOVED | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| TRN-23 | POST /coaching_sessions/S1/images | COACH, COACHEE | 200 (201), image | `web/src/extractors/coaching_session_access.rs:193` | |
| TRN-24 | POST /coaching_sessions/S1/images | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| TRN-25 | GET /coaching_session_images/IMG1 | COACH, COACHEE | 302 to storage, or 200 bytes | `web/src/extractors/coaching_session_image_access.rs:55` | |
| TRN-26 | DELETE, POST restore /coaching_session_images/IMG1 | COACH, COACHEE | 200, IMG1 | `web/src/extractors/coaching_session_image_access.rs:55` | Either participant, not only the uploader |
| TRN-27 | GET, DELETE, POST restore /coaching_session_images/IMG1 | non-participants | 403 | `web/src/extractors/coaching_session_image_access.rs:55` | Unknown image: 404 |
| TRN-28 | POST /coaching_sessions/S3/transcript_suggestions/SG3/accept | COACH | 409 `assignee_not_a_member`, no action written, no email | `domain/src/transcript_suggestion/review.rs:339` | [membership] |
| TRN-29 | any transcription, suggestion, recording or image route | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |
| TRN-30 | Replaced by TRN-33: a colleague transcript is never given an analysis, so the sweep has none to skip | | | | |
| TRN-31 | GET, POST /coaching_sessions/{RC session}/transcript_suggestions/scan, RC the KIND-04 colleague relationship, its session with a completed transcript | COACH, MEMBER | GET 200 `status: none`; POST 202 `status: pending`, one pending analysis written | `domain/src/transcript_suggestion/review.rs:515` | Colleague sessions are scanned like coaching ones |
| TRN-32 | Transcript completion for a Google Meet session on RC whose link MEMBER made (`meeting_url_created_by_user_id` = MEMBER_ID) | operator | the attendee list is read with MEMBER's Google token and matched against COACH's Google account; with no recorded link maker, both are COACH's | `domain/src/transcription.rs` `gather_evidence` | Who is attributed is unchanged |
| TRN-33 | Transcript completion for RC's session with suggestions configured, beside one for R1 | operator | RC's transcript gets no `transcript_analyses` row; R1's gets one `pending` (psql) | `domain/src/transcription.rs` `owed_analyses` | Nothing is queued that the sweep would never claim. Replaces TRN-30 |
| TRN-34 | Two near-simultaneous POST /coaching_sessions/S1/meeting_recording `{meeting_url}` against Postgres, with `RECALL_AI_BASE_URL` pointed at a local mock of Recall | COACH and COACHEE at once | one 200 (201), one 409 `recording_already_active`; one `meeting_recordings` row for S1 that is not terminal (psql) and one bot-create request in the mock's log (or one `Recording bot ... created` backend log line) | `domain/src/meeting_recording.rs` `start`; the index `idx_meeting_recordings_active_per_session` | Never against Recall: local and production keys share one workspace |
| TRN-35 | With O1 archived (ORG-33): POST /coaching_sessions/S1/meeting_recording `{meeting_url}`, with `RECALL_AI_BASE_URL` pointed at a local mock of Recall | COACH | 409, `error` `organization_archived`, no `details`; no new `meeting_recordings` row for S1 (psql) and no bot-create request in the mock's log | `domain/src/meeting_recording.rs` `start`, checked before the claim | Never against Recall. Unarchive O1 afterwards (ORG-35) |

### 4.9 Meeting links and OAuth-dependent flows (`MEET`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| MEET-01 | GET /coaching_sessions/SR1-x (not yet hydrated) | COACHEE | 200, session with provider and `meeting_url` from COACH's connection | `web/src/extractors/coaching_session_access.rs:193`; `domain/src/coaching_session.rs:268` | Hydration always uses the coach's account, whoever reads first; the link creator recorded is COACH (SCHED-09) |
| MEET-02 | POST /coaching_sessions with `provider` | caller per SES-09 to SES-14 | as SES-09 to SES-14; a link is attached only on SES-09 | `domain/src/coaching_session.rs:116` | [tightened] Link comes from the coach's account or a reused relationship link (`:648`) |
| MEET-03 | PUT /coaching_sessions/S1 `{meeting_url, provider}` | COACH | 200 (204) | `web/src/extractors/coaching_session_access.rs:98` | Coach only; COACHEE 403 (SES-16) |
| MEET-04 | POST /coaching_sessions/S1/meeting_recording `{meeting_url}` | COACHEE | 200 (201) | `web/src/extractors/coaching_session_access.rs:193` | URL is caller supplied; not tied to an account |
| MEET-05 | GET /oauth/connections, GET/DELETE /oauth/connections/{provider} | any authenticated caller | 200, caller's own connections | `web/src/router.rs:802`, `:804` (auth only) | Self scoped, no relationship data. Listed so MEET-01 has its prerequisite |
| MEET-06 | GET /oauth/{provider}/authorize, GET /oauth/connections, GET/DELETE /oauth/connections/{provider} | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | The provider's callback takes no session |

### 4.10 Search (`SRCH`)

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
| SRCH-11 | GET /search?q=kumquat&types=actions | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |

### 4.11 Collaboration token (`JWT`)

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| JWT-01 | GET /jwt/generate_collab_token?coaching_session_id=S1 | COACH, COACHEE | 200, token for S1's document | `web/src/extractors/coaching_session_access.rs:193` | |
| JWT-02 | GET /jwt/generate_collab_token?coaching_session_id=S1 | non-participants | 403 | `web/src/extractors/coaching_session_access.rs:193` | Unknown session: 404 |
| JWT-03 | GET /jwt/generate_collab_token?coaching_session_id=S3 | REMOVED | 403 | `web/src/extractors/coaching_session_access.rs:193` | |
| JWT-04 | GET /jwt/generate_collab_token?coaching_session_id=S1 | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |

### 4.12 Users (`USR`)

Rows here cover user routes that return relationship data and are not covered above
(`REL-13`, `REL-14`, `REL-17`, `SES-27` to `SES-32`, `SES-43`, `ACT-18` to `ACT-26`,
`GOAL-22` to `GOAL-24`, `GOAL-34`).

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| USR-01 | GET /users/{COACHEE_ID}/coaching_sessions?include=relationship | COACHEE | 200, S1 S2 SR1-* each with `relationship` and `participants`, COACHEE second | `web/src/extractors/user_self_access.rs:28`; `entity_api/src/coaching_session.rs:1178`, `:1366` | [rows] |
| USR-02 | GET /users/{COACHEE_ID}/coaching-relationships?role=coach | COACHEE | 200, empty list | `entity_api/src/coaching_relationship.rs:662` | [rows] |
| USR-03 | GET /users/{COACH_ID}/coaching-relationships?role=coach | COACH | 200, R1 R3 | `entity_api/src/coaching_relationship.rs:662` | [rows] |
| USR-04 | any /users route | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |
| USR-05 | PUT /users/{COACHEE_ID} `{working_hours_start}` | COACH, MEMBER, ADMIN, SUPER | 403, COACHEE unchanged | `web/src/extractors/user_self_access.rs:28` | Self only |
| USR-06 | GET /users?email={MEMBER email} | COACH, COACHEE, MEMBER, OUTSIDER | 403 | `web/src/controller/user_controller.rs:57` | Only an org `Admin` or SUPER may look users up |
| USR-07 | GET /users?email={MEMBER email} | ADMIN | 200, MEMBER | `domain/src/user_role.rs:279`; `entity_api/src/user_role.rs:331` | [rows] the lookup's rate limit takes a Postgres advisory lock, so the route is checked by hand and the scope in `domain` |
| USR-08 | GET /users?email={OUTSIDER email} | ADMIN | 200, empty list, also once OUTSIDER has role history in O2 | `domain/src/user_role.rs:279`; `entity_api/src/user_role.rs:331`; `entity_api/src/user_role_change.rs:58` | [rows] Not found and not visible look the same |
| USR-09 | GET /users?email={REMOVED email} after ORG-04 | ADMIN | 200, REMOVED; ADMIN may also attach them (ORG-03) | `domain/src/user_role.rs:279`, `:297`; `entity_api/src/user_role_change.rs:58` | [rows] Without O1 role history: empty list |
| USR-10 | GET /users?email={any fixture email} | SUPER | 200, that user | `domain/src/user_role.rs:279` | [rows] |

### 4.13 Organizations (`ORG`)

Organization membership changes decide `is_participant_and_member`; these rows create or end it.

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| ORG-01 | POST /organizations/O1/users `{..., relationship: {with_user_id: COACH, kind: "coaching"}}` | ADMIN, SUPER | 200 (201), new user plus a relationship in which COACH coaches the new user | `web/src/extractors/organization_admin_access.rs:42`; `domain/src/user.rs:156` | Sends MAIL-12. `kind: "colleague"` makes the two colleagues instead (KIND-15) |
| ORG-02 | POST /organizations/O1/users | COACH, COACHEE, MEMBER, OUTSIDER | 403 | `web/src/extractors/organization_admin_access.rs:48` | |
| ORG-03 | POST /organizations/O1/users/{user}/role `{role: User, relationship: {with_user_id: COACH, kind: "coaching"}}` | ADMIN, SUPER | 200 (201), membership plus a relationship in which COACH coaches the user | `web/src/controller/organization/user/role_controller.rs:82`; `domain/src/user_role.rs:79` | Target must be administrable by the caller, else 404. Sends MAIL-13. `kind: "colleague"` makes the two colleagues instead (KIND-16) |
| ORG-04 | DELETE /organizations/O1/users/{REMOVED_ID}/role | ADMIN, SUPER | 200 (204) | `web/src/extractors/organization_admin_access.rs:42`; `domain/src/user_role.rs:107` | R3 is kept; REMOVED loses every `is_participant_and_member` route |
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
| ORG-25 | POST /organizations `{name}` | SUPER | 200 (201), new organization | `web/src/extractors/super_admin_access.rs:27` | PUT, DELETE, archive and unarchive each take the same extractor: ORG-29 to ORG-36 |
| ORG-26 | POST /organizations | ADMIN, COACH, MEMBER, OUTSIDER | 403, nothing written | `web/src/extractors/super_admin_access.rs:29` | |
| ORG-27 | POST /organizations | OUTSIDER holding `SuperAdmin` scoped to O2 | 403, nothing written | `web/src/extractors/super_admin_access.rs:27`; `entity/src/users.rs:95` | Only a `SuperAdmin` role with no organization counts. The model refuses to save a scoped one, so set it in the database by hand |
| ORG-28 | any /organizations route | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |
| ORG-29 | PUT /organizations/O1 `{name}` | SUPER | 200, O1 renamed | `web/src/controller/organization_controller.rs:148`; `web/src/extractors/super_admin_access.rs:27` | Name or slug taken by another organization: 409 |
| ORG-30 | PUT /organizations/O1 `{name}` | ADMIN, COACH, MEMBER, OUTSIDER; OUTSIDER holding `SuperAdmin` scoped to O2 | 403, O1 unchanged | `web/src/controller/organization_controller.rs:148`; `web/src/extractors/super_admin_access.rs:29`; `entity/src/users.rs:95` | |
| ORG-31 | DELETE /organizations/{id} (an organization with no members and no relationships) | SUPER | 200, `{id}`; the organization is gone | `web/src/controller/organization_controller.rs:188`; `web/src/extractors/super_admin_access.rs:27` | With members or relationships: 409 |
| ORG-32 | DELETE /organizations/{id} (the same empty organization) | ADMIN, COACH, MEMBER, OUTSIDER; OUTSIDER holding `SuperAdmin` scoped to O2 | 403, the organization is kept | `web/src/controller/organization_controller.rs:188`; `web/src/extractors/super_admin_access.rs:29`; `entity/src/users.rs:95` | |
| ORG-33 | POST /organizations/O1/archive | SUPER | 200, O1 archived | `web/src/controller/organization_controller.rs:216`; `web/src/extractors/super_admin_access.rs:27` | Idempotent. Unarchive afterwards (ORG-35) |
| ORG-34 | POST /organizations/O1/archive | ADMIN, COACH, MEMBER, OUTSIDER; OUTSIDER holding `SuperAdmin` scoped to O2 | 403, O1 not archived | `web/src/controller/organization_controller.rs:216`; `web/src/extractors/super_admin_access.rs:29`; `entity/src/users.rs:95` | |
| ORG-35 | POST /organizations/O1/unarchive (O1 archived) | SUPER | 200, O1 no longer archived | `web/src/controller/organization_controller.rs:246`; `web/src/extractors/super_admin_access.rs:27` | Idempotent |
| ORG-36 | POST /organizations/O1/unarchive (O1 archived) | ADMIN, COACH, MEMBER, OUTSIDER; OUTSIDER holding `SuperAdmin` scoped to O2 | 403, O1 stays archived | `web/src/controller/organization_controller.rs:246`; `web/src/extractors/super_admin_access.rs:29`; `entity/src/users.rs:95` | Unarchive as SUPER afterwards |
| ORG-37 | GET /organizations/O1 | COACH, COACHEE, OTHER_COACH, MEMBER, ADMIN, SUPER | 200, O1 | `web/src/extractors/organization_member_access.rs:81`, `:86` | |
| ORG-38 | GET /organizations/O1 | OUTSIDER, REMOVED; OUTSIDER holding `SuperAdmin` scoped to O2 | 403 | `web/src/extractors/organization_member_access.rs:91`; `entity/src/users.rs:95` | Not a member of O1 |
| ORG-39 | GET /organizations/{unknown} | SUPER, OUTSIDER | 404 | `web/src/extractors/organization_member_access.rs:59` | The existence check runs before the membership check, as on every route with this extractor (REL-19) |
| ORG-40 | POST /organizations/O1/users with the replaced `coach_id` beside `relationship` | ADMIN | 422, the unknown-field rejection; no user, no email | `web/src/params/user/mod.rs` `CreateMemberParams` | The same rejection an unknown field in a role body gets |
| ORG-41 | POST /organizations/O1/users `{..., relationship: {with_user_id: COACHEE, kind: "coaching", role: "coach"}}` | ADMIN | 200 (201), new user plus a relationship in which the new user coaches COACHEE; sends MAIL-12 once | `domain/src/coaching_relationship.rs` `InviteeRelationship::seated`; `domain/src/user.rs` `create_in_organization` | `relationship.role` is the new member's role in the relationship, not their organization role. Delete the user afterwards |
| ORG-42 | POST /organizations/O1/users with `relationship: {kind: "coaching", role: "colleague"}`, then `{kind: "colleague", role: "coach"}` | ADMIN | 422 for both, `details.reason` `role_not_in_kind`; no user, no email | `domain/src/coaching_relationship.rs` `InviteeRelationship::seated` | Refused before anything is written |
| ORG-43 | POST /organizations/O1/users/{user}/role `{role: User, relationship: {with_user_id: COACHEE, kind: "coaching", role: "coach"}}` (a former member with no relationship) | ADMIN | 200 (201), membership plus a relationship in which the user coaches COACHEE | `domain/src/coaching_relationship.rs` `InviteeRelationship::seated`; `domain/src/user_role.rs` `attach_to_organization` | The top-level `role` is the organization role |
| ORG-44 | POST /organizations/O1/users/{REMOVED_ID}/role `{role: User, relationship: {with_user_id: COACH, kind: "coaching", role: "coach"}}`, then the same with `kind: "colleague", role: "colleague"` | ADMIN | first: 200 (201), REMOVED now coaches COACH and R3 is kept (they coach each other); second: 409 `pair_already_related`, no role granted | `entity_api::coaching_relationship::create` | Run each on a fresh REMOVED removal; delete the new relationship afterwards |
| ORG-45 | POST /organizations/O1/users `{..., relationship: {with_user_id: COACH, kind: "colleague"}}` | ADMIN | 200 (201); O1's audit trail ends with `member_added` (subject the new user, `details.role` `user`) then `relationship_created`, both by ADMIN | `domain/src/user.rs:164`, `:170` | |
| ORG-46 | KIND-18 | ADMIN | 409; no audit row | `domain/src/user.rs:164` | The `member_added` recorded before the refusal rolls back with it |
| ORG-47 | ORG-03 with `kind: "colleague"` for a former member with no relationship | ADMIN | 200 (201); `member_added` then `relationship_created`, both by ADMIN | `domain/src/user_role.rs:87`, `:92` | |
| ORG-48 | ORG-03 for REMOVED, whose R3 is reused | ADMIN | 200 (201); only `member_added` | `domain/src/coaching_relationship.rs:133` | |
| ORG-49 | KIND-17 | ADMIN | 409; no audit row | `domain/src/user_role.rs:87` | |
| ORG-50 | ORG-04 | ADMIN | `member_removed` by ADMIN, subject the user, `details` `{}` | `domain/src/user_role.rs:163` | |
| ORG-51 | ORG-24 | ADMIN | `member_role_changed`, `details` `{from: "user", to: "admin"}` | `domain/src/user_role.rs:266` | |
| ORG-52 | ORG-06 for a member in a relationship without sessions | ADMIN | `relationship_deleted` by ADMIN per deleted relationship, in the relationship's organization, `details` its `kind`, `coach_id`, `coachee_id` | `domain/src/user.rs:260` | |
| ORG-53 | ORG-06 | ADMIN | after any `relationship_deleted`, `member_removed` by ADMIN in each organization the user belonged to, `details` `{}` | `domain/src/user.rs:271` | The delete is refused for a member of two organizations, so there is at most one |
| ORG-54 | Ten POST /organizations/O1/users at once, each with its own email (`seq 10 \| xargs -P 10 -I{} curl ...`) | ADMIN | all 200 (201); ten more `member_added`; the fork check below returns no rows | `entity_api/src/audit_event.rs:57` | Postgres only: SQLite has no advisory lock, so no automated test sees two appends race. Sends ten invites, so run it against the mock Resend |

The audit rows' checks (each organization's `prev_hash` is its previous event's `hash`):

```sql
SELECT actor_user_id, action, subject_type, subject_id, details, prev_hash, hash
FROM refactor_platform.audit_events WHERE organization_id = '<O1>' ORDER BY occurred_at, id;
```

Two events naming the same `prev_hash` mean two appends forked the chain (ORG-54):

```sql
SELECT prev_hash, count(*) FROM refactor_platform.audit_events
WHERE organization_id = '<O1>' GROUP BY prev_hash HAVING count(*) > 1;
```

### 4.14 Real-time events (`SSE`)

`GET /sse` opens a stream for the caller; events go to the user ids each publisher computes.
Check with two browsers or `curl -N -b /tmp/<jar>.jar -H "$VER" $BASE/sse`.

| ID | Method and route | Caller | Expected | Enforced at (file:line) | Notes |
|---|---|---|---|---|---|
| SSE-01 | GET /sse | any authenticated caller | 200, event stream | `web/src/sse/handler.rs:12` | No relationship check; delivery is per user id |
| SSE-02 | POST /actions in S1 (assigned to COACH), then PUT its status, then DELETE it | streams of COACH, COACHEE | both get `action_created`, then `action_updated`, each with the full action, then `action_deleted` with its id | `domain/src/action.rs:69`, `:83`, `:174`; `entity_api/src/coaching_relationship.rs:198` | [rows] coachee receives coach-assigned actions here |
| SSE-03 | any action, goal, link, title or topic change in R3 | stream of REMOVED | nothing delivered | `entity_api/src/coaching_relationship.rs:198` | [rows] recipients filtered to current org members |
| SSE-04 | goal create/update in R1 | streams of COACH, COACHEE | both get the goal event | `domain/src/goal.rs:130` | [rows] |
| SSE-05 | transcript suggestions ready for S1 (JOB-03) | streams of COACH, COACHEE | both get `transcript_suggestions_changed` | `domain/src/transcript_suggestion/extract.rs:240` | [rows] |
| SSE-06 | POST /webhooks/recall_ai (recording or transcript update for S1) | streams of COACH, COACHEE | both get the recording/transcription event | `entity_api/src/coaching_session.rs:655` | [rows] webhook is signature-authenticated, no user |
| SSE-07 | GET /sse | not signed in | 401 | `web/src/middleware/auth.rs:25`; `web/src/extractors/authenticated_user.rs:28` | |

### 4.15 Who scheduled and per-person notice (`SCHED`)

Phase 1 records, on every schedule change, who made it, whose account made the meeting link,
and when each participant (not the session as a whole) was last told its start. Nothing a
caller sees changes: these rows check what is stored, with psql, not who may do it (the
`SES`, `SER` and `MEET` rows above still decide that). Colleagues will make the recorded
scheduler differ from the coach; until phase 4 every value written here is the caller who
passed those rows, which is always COACH.

Read a session's record, and its notice rows:

```sql
SELECT scheduled_by_user_id, meeting_url_created_by_user_id, meeting_url IS NOT NULL AS has_link
FROM refactor_platform.coaching_sessions WHERE id = '<session id>';

SELECT user_id, notice_given_at FROM refactor_platform.coaching_session_notices
WHERE coaching_session_id = '<session id>' ORDER BY user_id;
```

To see a row change rather than keep the coach it already held, point it at someone else first
(it can't be null):
`UPDATE refactor_platform.coaching_sessions SET scheduled_by_user_id = '<MEMBER_ID>' WHERE id = '<id>';`

`JOB-02` rows need the reschedule email to settle quickly: run the backend with
`SESSION_RESCHEDULE_SETTLE_SECONDS=30`, and use a mock Resend (`RESEND_BASE_URL`) unless every
inbox is yours.

| ID | Action | Caller | Expected (psql) | Recorded by | Notes |
|---|---|---|---|---|---|
| SCHED-01 | POST /coaching_sessions `{coaching_relationship_id: R1}` (SES-09) | COACH | `scheduled_by_user_id` = COACH_ID; two notice rows, COACH and COACHEE, both at the same time, the moment of the create | `domain::coaching_session::create` | |
| SCHED-02 | POST /coaching_sessions `{coaching_relationship_id: R1, scheduled_by_user_id: MEMBER_ID, meeting_url_created_by_user_id: MEMBER_ID}`, no `provider` | COACH | 200 (201); `scheduled_by_user_id` = COACH_ID, `meeting_url_created_by_user_id` null; response has neither field | `entity::coaching_sessions::Model` (`#[serde(skip)]`) | A caller can't name someone else as the scheduler |
| SCHED-03 | POST /coaching_sessions `{coaching_relationship_id: R1, provider}` (MEET-02) | COACH | `meeting_url_created_by_user_id` = COACH_ID when a link was attached; null when none was | `domain::coaching_session::maybe_attach_meeting_url` | A new link records the coach; a reused one keeps the creator of the session it came from, or the coach when that has none |
| SCHED-04 | PUT /coaching_sessions/S1 moving `date`, on a row set to MEMBER_ID | COACH | `scheduled_by_user_id` = COACH_ID | `domain::coaching_session::update` | Written in the edit's own transaction |
| SCHED-05 | PUT /coaching_sessions/S1 changing only `title`, on a row set to MEMBER_ID | COACH | `scheduled_by_user_id` still MEMBER_ID; notice rows unchanged | `domain::coaching_session::update` | Only a create or a schedule change records a scheduler |
| SCHED-06 | POST /coaching_session_series `{coaching_relationship_id: R1}` (SER-01) | COACH | every new session: `scheduled_by_user_id` = COACH_ID, two notice rows | `domain::coaching_session_series::create_with_sessions` | |
| SCHED-07 | PUT /coaching_session_series/SR1 (SER-08) | COACH | every session it created or moved: `scheduled_by_user_id` = COACH_ID; past sessions untouched | `domain::coaching_session_series::reschedule` | Clear the future SR1 sessions first |
| SCHED-08 | POST /coaching_session_series/SR1/reschedule from an occurrence (SER-10) | COACH | moved and newly created sessions: COACH_ID; sessions before that occurrence keep their value (set one to MEMBER_ID first to see it stay) | `domain::coaching_session_series::reschedule_following` | |
| SCHED-09 | GET /coaching_sessions/SR1-x not yet hydrated (MEET-01) | COACHEE | link attached; `meeting_url_created_by_user_id` = COACH_ID, not the reader; `scheduled_by_user_id` unchanged | `domain::coaching_session::ensure_hydrated`; `entity_api::coaching_session::mark_hydrated` | |
| SCHED-10 | Reschedule email delivered to both (JOB-02 after SCHED-04) | COACH, COACHEE | both notice rows restamped to the send time | the reschedule settle job | Compare against the values before the PUT |
| SCHED-11 | PUT /coaching_sessions/S1 moving `date`, then remove COACH from O1 (ORG-04) before the email settles | COACH | only COACHEE is emailed; COACHEE's notice row restamped, COACH's unchanged | the reschedule settle job | Per-person: a participant who got no email keeps their old stamp. Run on a disposable relationship, or re-add COACH after |
| SCHED-12 | PUT /coaching_sessions/S1 moving `date` with every send failing (`touch /tmp/resend_fail` on the mock) | COACH | both notice rows restamped when the emails are written; the outbox retries the sends; the move itself saved | the reschedule settle job | Remove the file afterwards |
| SCHED-13 | GET /coaching_sessions/S1, and the OpenAPI document (`/api-docs/openapi2.json`, or `/rapidoc`) | COACH | no `scheduled_by_user_id`, `meeting_url_created_by_user_id` or `notice_given_at` anywhere | `entity::coaching_sessions::Model` | Phase 1 adds nothing to a response; 1c removes `notice_given_at`, which the frontend never read |
| SCHED-14 | Phase 1 migrations on a database that predates them | operator | every session's scheduler is its coach (NOT NULL); link creator is the coach exactly where `meeting_url` is set; a notice row per participant; `coaching_sessions.notice_given_at` gone | `migration/src/m20261114_00000{0,1,2,4,5}*` | For a fresh local database. The 1c migration's settle and its down: reminder plan H5b |
| SCHED-15 | PUT /coaching_sessions/S1 `{meeting_url: "https://meet.example/kumquat"}` (MEET-03), then again with the same URL | COACH | after the first PUT: `meeting_url_created_by_user_id` = COACH_ID; the repeat PUT leaves it as it was. The PUT cannot clear a link: an omitted or null `meeting_url` leaves it unchanged | `domain::coaching_session::update` | A link set by hand records whoever set it. Clear the creator to null first to see the first PUT write it |
| SCHED-16 | POST /coaching_sessions `{coaching_relationship_id: R1, meeting_url: "https://meet.example/kumquat"}`, no `provider` | COACH | `meeting_url_created_by_user_id` = COACH_ID | `domain::coaching_session::create` | A pasted link, not one made by a connected account |
| SCHED-17 | POST /coaching_sessions `{coaching_relationship_id: RC}`, no `duration_minutes` | MEMBER | `duration_minutes` = MEMBER's `default_coaching_session_duration_minutes`, not COACH's | `domain::coaching_session::create` | Set the two defaults apart first. Coaching (SES-09) keeps the coach's default whoever is recorded |
| SCHED-18 | POST /coaching_sessions `{coaching_relationship_id: RC, provider: google}`, MEMBER with no Google connection, COACH with one, RC with no earlier link | MEMBER | link attached; `meeting_url_created_by_user_id` = COACH_ID. With neither connected: no link, creator null | `domain::coaching_session::maybe_attach_meeting_url` | The scheduler's account first, then the other colleague's. A reused RC link keeps its creator, or the scheduler when it has none |
| SCHED-19 | POST /coaching_sessions `{coaching_relationship_id: R1, provider: google}`, COACH with no Google connection, COACHEE with one, R1 with no earlier link | COACH | no link; `meeting_url_created_by_user_id` null | `domain::coaching_session::maybe_attach_meeting_url` | Coaching never uses the coachee's account |
| SCHED-20 | POST /coaching_sessions `{coaching_relationship_id: RC}` (MAIL-01) | MEMBER | both invites' `DTSTART;TZID` is `America/New_York`, MEMBER's zone | `domain::emails::Participants::schedule_owner` | Zones as SER-25. Its cancellation and reschedule invites use the same zone |
| SCHED-21 | GET /coaching_sessions/{SRC session} not yet hydrated, SRC a series on RC that MEMBER created, after MEMBER's O1 role is removed (ORG-04); MEMBER and COACH both with a Google connection, RC with no earlier link | COACH | link attached from COACH's account; `meeting_url_created_by_user_id` = COACH_ID; MEMBER's token is never read or refreshed. With COACH unconnected: no link, the read still 200. On R1 with COACH removed: no link | `domain::coaching_session::ensure_hydrated`; `domain::coaching_session::member_meeting_account_ids` | A removed participant's account is never used for a new link. Reusing an existing persistent link needs a connected member to pick the provider |
| SCHED-22 | POST /coaching_sessions `{coaching_relationship_id: RC}`, no `provider` and no `meeting_url`, MEMBER with no meeting connection, COACH with a Google one, RC with no earlier link | MEMBER | link attached; `provider` google; `meeting_url_created_by_user_id` = COACH_ID. With MEMBER connected too: no link, `provider` null. On R1 as COACH, coach unconnected and COACHEE connected: no link | `domain::coaching_session::create`; `domain::coaching_session::other_account_provider` | A scheduler who can't see the other colleague's connections sends no provider; a connected scheduler who sends none gets none |

SCHED-14's checks, each expected to return 0:

```sql
SELECT count(*) FROM refactor_platform.coaching_sessions cs
JOIN refactor_platform.coaching_relationships cr ON cr.id = cs.coaching_relationship_id
WHERE cs.scheduled_by_user_id IS DISTINCT FROM cr.coach_id
   OR cs.meeting_url_created_by_user_id IS DISTINCT FROM
      CASE WHEN cs.meeting_url IS NOT NULL THEN cr.coach_id END;

SELECT count(*) FROM refactor_platform.coaching_sessions cs
JOIN refactor_platform.coaching_relationships cr ON cr.id = cs.coaching_relationship_id
WHERE (SELECT count(*) FROM refactor_platform.coaching_session_notices n
       WHERE n.coaching_session_id = cs.id)
      <> CASE WHEN cr.coach_id = cr.coachee_id THEN 1 ELSE 2 END;
```

Run these right after the migrations, before any new writes. 1c fills in any session written
between the 1a migration and 1b's code, so both hold on a database that ran the phases apart.

### 4.16 Relationship kind and role (`KIND`)

Phase 2 gives each relationship a `kind` (`coaching` or `colleague`) and names what a participant
is (`coach`, `coachee` or `colleague`) with one type. Phase 4 lets the relationship POST set
`kind` (omitted, it is `coaching`), and the two routes adding a person to an organization
(ORG-01, ORG-03) take an optional `relationship: {with_user_id, kind}`: for `coaching`, `with_user_id` coaches the person
added; for `colleague`, the two are colleagues. A refused relationship adds no person, grants
no role and sends no email. Every path that creates a relationship passes
its kind to the one create, and none assumes an invited person is coached. That create keeps
one kind per pair in an organization: colleagues are an unordered pair, so their relationship
is returned whichever person is named first, and a request of the other kind for the pair, in
either direction, is a 409 with `details.reason` `pair_already_related` and
`details.relationship_id` the existing one. A coaching pair may still coach the other way
round. Who may do what
does not change: the `REL`, `SES` and `SER` rows above still decide that. Wire values are
lowercase.

| ID | Action | Caller | Expected | Decided at | Notes |
|---|---|---|---|---|---|
| KIND-01 | GET /organizations/O1/coaching_relationships (REL-01, REL-05) | COACH, ADMIN | 200; every relationship has `"kind": "coaching"` | `entity_api::coaching_relationship::CoachingRelationshipWithUserNames` | |
| KIND-02 | GET /organizations/O1/coaching_relationships/R1 (REL-08) | COACH | 200; `"kind": "coaching"` | same | |
| KIND-03 | GET /users/{COACHEE_ID}/coaching-relationships (REL-13) | COACHEE | 200; each has `"kind": "coaching"` | same | |
| KIND-04 | POST /organizations/O1/coaching_relationships `{coach_id: COACH, coachee_id: MEMBER, kind: "colleague"}` (REL-15) | ADMIN | 200 (201); response `"kind": "colleague"`; psql `kind = 'colleague'` | the handler passes the body's `kind` (default `coaching`) to `entity_api::coaching_relationship::create` | Delete the relationship afterwards |
| KIND-05 | POST /organizations/O1/users `{..., relationship: {with_user_id: COACH, kind: "coaching"}}` (ORG-01), and POST /organizations/O1/users/{user}/role `{role: User, relationship: {with_user_id: COACH, kind: "coaching"}}` (ORG-03) | ADMIN | each new relationship's `kind = 'coaching'` (psql) | `domain::user`, `domain::user_role`: each passes the requested kind to `entity_api::coaching_relationship::create` | Adding a person with a `relationship` creates it in the same transaction. A body with the old `coach_id` is refused on ORG-03 (422, unknown field) and ignored on ORG-01 (the member is added with no relationship) |
| KIND-10 | GET /users/{COACH_ID}/coaching_sessions?include=relationship&from_date=2000-01-01&to_date=2100-01-01 (SES-27) | COACH | each session's `relationship` has `"kind": "coaching"` | `entity_api::coaching_relationship::WithMembership` (flattens the relationship `Model`) | The relationship embedded in an enriched session |
| KIND-06 | POST /coaching_sessions (SES-09) with a mock Resend | COACH | the scheduled email's `other_user_role` is `"coach"` in COACHEE's copy and `"coachee"` in COACH's, as before | `entity::coaching_relationship_kind::Role::as_str`; `domain/src/emails.rs` | The role words come from the one `Role` type |
| KIND-07 | Transcript download and speakers (`transcript_download_manual_testing.md` cases 1 to 5c) | COACH | role values `"coach"` / `"coachee"` and filenames unchanged; `speaker=colleague` per case 5c | `domain::transcript_export` | |
| KIND-08 | The OpenAPI document (`/api-docs/openapi2.json`, or `/rapidoc`) | COACH | `CoachingRelationshipWithUserNames` lists `kind` as required; schema `domain.coaching_relationship_kind.Kind` has `coaching`, `colleague`; schema `domain.coaching_relationship_kind.Role` has `coach`, `coachee`, `colleague`; no `SpeakerRole` | `entity::coaching_relationship_kind` | |
| KIND-09 | Phase 2 migration on a database at `m20261114_000005` | operator | every relationship `kind = 'coaching'`; `kind` NOT NULL, default `'coaching'`; the type is owned by `refactor`; `migrate down -n 1` removes the column and the type | `migration/src/m20261115_000000_add_kind_to_coaching_relationships.rs` | |
| KIND-11 | POST /organizations/O1/coaching_relationships `{coach_id: COACH, coachee_id: MEMBER, kind: "colleague"}`, then `{coach_id: MEMBER, coachee_id: COACH, kind: "colleague"}` | ADMIN | both 200 (201) with the same `id`; psql one row for the pair | `entity_api::coaching_relationship::create` (`settle`) | Colleagues are an unordered pair. Delete the relationship afterwards |
| KIND-12 | POST /organizations/O1/coaching_relationships `{coach_id: COACH, coachee_id: MEMBER, kind: "colleague"}`, and again with COACH and MEMBER swapped, while COACH coaches MEMBER | ADMIN | each 409, `details.reason` `pair_already_related`, `details.relationship_id` the coaching relationship; no new row | same | Create the coaching relationship first; delete it afterwards |
| KIND-13 | POST /organizations/O1/coaching_relationships `{coach_id: COACH, coachee_id: MEMBER}`, and again with COACH and MEMBER swapped, while the two are colleagues | ADMIN | each 409, `details.reason` `pair_already_related`, `details.relationship_id` the colleague relationship; no new row | same | Create the colleague relationship first; delete it afterwards |
| KIND-14 | POST /organizations/O1/coaching_relationships `{coach_id: MEMBER, coachee_id: COACH}` while COACH coaches MEMBER, then `{coach_id: COACH, coachee_id: MEMBER}` again | ADMIN | 200 (201) with a second relationship, `"kind": "coaching"`; then the first relationship's `id` | same | Unchanged from before phase 4. Delete both afterwards |
| KIND-15 | POST /organizations/O1/users `{..., relationship: {with_user_id: COACH, kind: "colleague"}}` (ORG-01) with a mock Resend | ADMIN | 200 (201); psql one relationship for the pair, `kind = 'colleague'`, COACH in `coach_id`, the new user in `coachee_id`; the welcome email (MAIL-12) is sent | `domain::user::create_in_organization` (`InviteeRelationship::draft_for`) | Delete the user afterwards |
| KIND-16 | POST /organizations/O1/users/{user}/role `{role: User, relationship: {with_user_id: COACH, kind: "colleague"}}` (ORG-03) for a former member with no relationship to COACH | ADMIN | 200 (201); membership granted; psql one relationship for the pair, `kind = 'colleague'` | `domain::user_role::attach_to_organization` | Remove the membership and the relationship afterwards |
| KIND-17 | POST /organizations/O1/users/{REMOVED}/role `{role: User, relationship: {with_user_id: COACH, kind: "colleague"}}` while COACH coaches REMOVED (R3) | ADMIN | 409, `details.reason` `pair_already_related`, `details.relationship_id` R3; REMOVED holds no O1 role; no new relationship; no MAIL-13 | `domain::user_role::attach_to_organization` (one transaction) | Remove REMOVED's O1 role first (ORG-04) |
| KIND-18 | POST /organizations/O1/users `{..., relationship: {with_user_id: OUTSIDER, kind: "colleague"}}` (OUTSIDER is not in O1), with a mock Resend | ADMIN | 409; no user with that email; no relationship; no welcome email | `domain::user::create_in_organization` (one transaction) | A new user has no relationship yet, so this route cannot hit `pair_already_related`; any refused relationship takes the same path |
| KIND-19 | GET /organizations/O1/coaching_relationships/R1 (REL-08) | COACH, COACHEE | 200; COACH: `"viewer": {"role": "coach", "can_schedule_sessions": true, "sees_all_actions": true}`; COACHEE: `{"role": "coachee", "can_schedule_sessions": false, "sees_all_actions": false}`; both `"suggests_actions": true` | `entity::coaching_relationships::viewer_of` (`Model::role`, `SchedulingPermit::verify`, `Model::sees_all_actions`); `entity_api::coaching_relationship::Capabilities` | Display only: no access check reads these fields |
| KIND-20 | GET /organizations/O1/coaching_relationships/RC, RC a colleague relationship | each colleague | 200; `"viewer": {"role": "colleague", "can_schedule_sessions": true, "sees_all_actions": true}`; `"suggests_actions": true` | same | |
| KIND-21 | GET /organizations/O1/coaching_relationships (REL-05) | ADMIN | 200; every relationship has `"viewer": null` | `viewer_of` is `None` for a caller in neither seat | Also SUPER on REL-06 |
| KIND-22 | A participant no longer in the organization | COACH after ORG-04 | `can_schedule_sessions: false` while `sees_all_actions` keeps its seat's answer | `SchedulingPermit::verify` checks membership | By unit test only: no read returns a relationship to a participant who left, unless a SuperAdmin, who keeps scheduling |
| KIND-23 | GET /users/{COACH_ID}/coaching_sessions?include=relationship (SES-27), and `/users/{COACH_ID}/coaching_sessions/nearest?...&include=relationship` | COACH | each session's `relationship` has COACH's `viewer`, `suggests_actions` and `scheduling_blocked_by` | `entity_api::coaching_session::EnrichedSession::for_caller` | |
| KIND-24 | GET /organizations/O1/coaching_relationships/R3 (REL-12); R1; RC after ORG-04 removes the colleague in the coach seat | COACH; COACH; the other colleague | R3: `"scheduling_blocked_by"` REMOVED's id; R1: `null`; RC: the removed colleague's id | `entity::coaching_relationships::Model::seat_blocking_scheduling`, shared with the scheduling 409 (`details.user_id`) | |
| KIND-25 | GET /users/{COACHEE_ID}/coaching-relationships (REL-13) | COACHEE | 200; each relationship has COACHEE's `viewer` | `web/src/controller/user/coaching_relationships_controller.rs` | |
| KIND-26 | POST /organizations/O1/coaching_relationships (REL-15) | ADMIN | 200 (201); `"viewer": null`, `"suggests_actions": true`, `"scheduling_blocked_by": null` | `web/src/controller/organization/coaching_relationship_controller.rs` | Delete the relationship afterwards |
| KIND-27 | KIND-11's two creates | ADMIN | the second response's `participants[0]` names COACH and `participants[1]` MEMBER, matching the stored `coach_id`/`coachee_id` | `entity_api::coaching_relationship::create` (`in_stored_seats`) | Names follow the stored seats, not the request's order. Delete the relationship afterwards |
| KIND-28 | PUT /organizations/O1/coaching_relationships/R1/kind `{kind: "colleague"}` | ADMIN | 409, `details.reason` `kind_switch_needs_confirmation`; `details.actions_becoming_visible` `[{user_id: COACHEE, by_assignee: [{assignee_id: COACH, count: 1}]}]` (A1); `details.scheduling_gained` `[COACHEE]`, `details.scheduling_lost` `[]`; psql R1 unchanged | `domain::coaching_relationship::kind_switch::switch_for_caller` (`consequences`, from each kind's rules) | No `confirmed: true`, no change |
| KIND-29 | KIND-28 with `confirmed: true`, then GET /organizations/O1/coaching_relationships/R1 as COACHEE | ADMIN, then COACHEE | 200, `"kind": "colleague"`, `"viewer": null`; psql `kind = 'colleague'`; COACHEE's `viewer` `{"role": "colleague", "can_schedule_sessions": true, "sees_all_actions": true}` | same; `entity_api::coaching_relationship::change_kind` | Switch R1 back afterwards (KIND-30 with `coach_id: COACH`) |
| KIND-30 | PUT /organizations/O1/coaching_relationships/RC/kind `{kind: "coaching", coach_id: <the colleague in coachee_id>, confirmed: true}`, RC a colleague relationship | ADMIN | 200, `"kind": "coaching"`; psql `coach_id` and `coachee_id` swapped | `kind_switch::target_of` | |
| KIND-31 | PUT /organizations/O1/coaching_relationships/R1/kind with `confirmed: true` and `{kind: "coaching", coach_id: MEMBER}`; `{kind: "coaching"}`; `{kind: "colleague", coach_id: COACH}` | ADMIN | each 422, `details.reason` `coach_not_a_participant`, `coach_required`, `coach_not_applicable`; psql R1 unchanged | `kind_switch::target_of` | |
| KIND-32 | PUT /organizations/O1/coaching_relationships/R1/kind `{kind: "coaching", coach_id: COACHEE}`, then again with `confirmed: true` | ADMIN | first 409 `kind_switch_needs_confirmation`, `scheduling_gained` `[COACHEE]`, `scheduling_lost` `[COACH]`, COACHEE gains sight of A1; then 200, psql R1's seats swapped | same | A reversal moves scheduling and visibility, so it needs confirming. Swap back afterwards |
| KIND-33 | With RB `{coach_id: COACHEE, coachee_id: COACH}` created (KIND-14): PUT /organizations/O1/coaching_relationships/R1/kind `{kind: "colleague", confirmed: true}`; RB the same; R1 `{kind: "coaching", coach_id: COACHEE, confirmed: true}` | ADMIN | each 409, `details.reason` `pair_already_related`, `details.relationship_id` the other of R1 and RB; nothing changed | `entity_api::coaching_relationship::ensure_pair_allows` (the create's `settle`), re-checked by `change_kind` under the pair's advisory lock | Delete RB afterwards |
| KIND-34 | PUT /organizations/O1/coaching_relationships/R1/kind `{kind: "colleague", confirmed: true}`; PUT /organizations/O2/coaching_relationships/R1/kind; PUT /organizations/O1/coaching_relationships/{unknown id}/kind | COACH, COACHEE, OTHER_COACH, MEMBER, OUTSIDER, REMOVED; SUPER; ADMIN | 403 each; 404; 404; nothing changed | `web/src/extractors/organization_admin_access.rs`; `entity_api::coaching_relationship::find_by_id_in_organization` | |
| KIND-35 | PUT /organizations/O1/coaching_relationships/R1/kind `{kind: "coaching", coach_id: COACH}` | ADMIN | 200, R1 as it is; psql `updated_at` unchanged | `kind_switch::switch_for_caller` | Nothing to switch, so no confirmation |
| KIND-36 | PUT /organizations/O1/coaching_relationships/R3/kind `{kind: "colleague"}`, and with `confirmed: true` | ADMIN | each 409, `details.reason` `participant_not_a_member`, `details.user_id` REMOVED, `details.organization_id` O1; psql R3 unchanged | `domain::coaching_relationship::ensure_members` | Checked before the confirmation step. Re-add the person first |
| KIND-37 | With a session in R1 starting within the reminder lead (default 24 hours): KIND-29, then wait for a reminder sweep tick, with a mock Resend | ADMIN | psql `coaching_session_reminders` holds a row for that session and COACH, `sent_for_start` its `date`; no row for S1; the sweep sends COACH no reminder for that session | `entity_api::coaching_session::mark_reminders_sent`, in the switch's transaction | The sweep's claim is Postgres-only SQL, so the automated test stops at the row; the send half is by hand |
| KIND-38 | With S4 (a content-free session in R1): KIND-29, then wait for a pending reschedule sweep tick (15 s), with a mock Resend; then another tick | ADMIN | psql `coaching_session_pending_invites` holds one row per (S1, S4, SR1) and (COACH, COACHEE), none per SR1 session; the tick mails each of COACH and COACHEE three invites (S1 and S4 standalone, SR1 as a series), each `.ics` under its old `UID` with a higher `SEQUENCE` and a `Colleagues:` DESCRIPTION; the rows are gone; the second tick mails nothing | `kind_switch::apply` (queued in the switch's transaction); `domain/src/jobs/coaching_session_pending_reschedule.rs` (`Sweep::resend_invite`) | One row per invite and participant, so each is sent once per participant; a delivered row is deleted |
| KIND-39 | KIND-29, then before the sweep ticks PUT R1 back `{kind: "coaching", coach_id: COACH, confirmed: true}`; then wait for a tick | ADMIN | psql still one row per (S1, SR1) and participant; the tick mails each participant two invites, not four | `entity_api::coaching_session_pending_invite::queue` (one row per target and user; owing it again releases any claim) | Each invite goes out as the relationship stands when sent |
| KIND-40 | KIND-29, then remove COACHEE from O1 before the sweep ticks | ADMIN | the tick mails COACH only (S1 and SR1); COACHEE's rows are deleted unsent | `domain::coaching_session::enqueue_owed_invite`, `domain::coaching_session_series::resend_invite_to` | The switch itself refuses non-members (KIND-36), so leaving afterwards is the only way here |
| KIND-41 | With a session in R1 two days in the past: KIND-29 | ADMIN | psql no `coaching_session_pending_invites` row for the past session | `kind_switch::apply` (`find_by_relationship_after` now) | |
| KIND-42 | With COACH's and COACHEE's `/sse` streams open: KIND-28 (no `confirmed`), then KIND-31's `coach_id: MEMBER` | ADMIN | 409, then 422; psql no `coaching_session_pending_invites` rows; neither stream receives `coaching_relationship_kind_changed` | `kind_switch::switch_for_caller` (queue and publish follow the confirmation and every check) | |
| KIND-43 | With COACH's, COACHEE's and MEMBER's `/sse` streams open: KIND-32 confirmed, then KIND-29 | ADMIN | COACH and COACHEE each receive one `coaching_relationship_kind_changed` per switch, data `{coaching_relationship_id: R1, organization_id: O1, from: "coaching", to: "coaching"}` (a reversal), then `to: "colleague"`; MEMBER receives none | `kind_switch::publish_kind_changed` (recipients: the participants still in O1); `sse/src/domain_event_handler.rs` | Published after the switch commits |
| KIND-44 | With O1 archived: PUT /organizations/O1/coaching_relationships/R1/kind `{kind: "colleague"}`, and with `confirmed: true` | ADMIN | each 409, `error` `organization_archived`; psql R1 unchanged, no `coaching_session_pending_invites` rows | `kind_switch::ensure_not_archived`, checked before the membership check, as `entity_api::coaching_relationship::create` refuses | Unarchive O1 afterwards |
| KIND-45 | GET /organizations/O1/coaching_relationships/R1; GET /organizations/O1/coaching_relationships; the same; GET /users/COACH/coaching-relationships; KIND-32 confirmed | COACH; ADMIN; COACH; COACH; ADMIN | 200 each; every relationship's `participants[0].email`/`participants[1].email` are the emails of its `coach_id`/`coachee_id` users (R1: COACH's, COACHEE's; after the reversal: COACHEE's, COACH's); KIND-27's second response carries COACH's and MEMBER's | `entity_api::coaching_relationship` (`with_user_names` and each `*_with_user_names` query) | Two users can share a name, so pickers show emails. Swap back afterwards |
| KIND-46 | KIND-30 (RC to coaching, `coach_id` the colleague in `coachee_id`, `confirmed: true`); then KIND-28 (no `confirmed`), KIND-31's `coach_id: MEMBER`, KIND-34 as COACH, and KIND-35 | ADMIN; ADMIN; ADMIN; COACH; ADMIN | the first 200 and O1's audit trail ends with one `relationship_kind_changed` by ADMIN, subject RC, `details` `{from: "colleague", to: "coaching", coach_id, coachee_id}` the seats after the switch; the rest 409, 422, 403, 200 and record nothing more | `domain/src/coaching_relationship/kind_switch.rs:328` | Audit checks below the `ORG` table; recorded last before the switch commits |
| KIND-47 | MAIL-01 to MAIL-03, MAIL-05 to MAIL-08 and MAIL-10 on R1, then on RC, against a mock Resend | COACH; for RC also MEMBER | every payload's `session_label` is `"coaching session"` for R1 and `"session"` for RC, and its `session_label_plural` `"coaching sessions"` for R1 and `"sessions"` for RC; the cancellation `.ics` DESCRIPTION reads `This coaching session has been cancelled.` and `This recurring coaching session series has been cancelled.` for R1, `This session has been cancelled.` and `This recurring session series has been cancelled.` for RC | `entity::coaching_relationship_kind::Rules::session_label`, `Rules::session_label_plural`; `domain/src/emails.rs` (`Participants::copies`, `Participants::cancelled_description`) | Additive variables: the templates switch to `{{{session_label}}}` and `{{{session_label_plural}}}` after the deploy |
| KIND-48 | GET /organizations/O1/coaching_relationships/R1; the same on RC, a colleague relationship; GET /organizations/O1/coaching_relationships/R3 | COACH; COACHEE; COACH | 200 each; `participants` is two objects `{id, first_name, last_name, email, role, is_member}`, the coach seat first: R1 `role` `coach` then `coachee`; RC `colleague` for both, its `coach_id` user first; R3 `is_member` true then false. None of `coach_first_name`, `coach_last_name`, `coachee_first_name`, `coachee_last_name`, `coach_email`, `coachee_email`, `coach_is_member`, `coachee_is_member` appear | `entity_api::coaching_relationship::CoachingRelationshipWithUserNames::participants` | `role` comes from the kind's rules for each seat. Also on the create, kind switch and `GET /users/{id}/coaching-relationships` responses |
| KIND-49 | GET /users/{COACH_ID}/coaching_sessions?include=relationship (SES-27); the same without `include`; GET /users/{COACHEE_ID}/coaching_sessions/nearest?coaching_relationship_ids=R1&around={now}&before=1&after=1&include=relationship with R1 made a colleague relationship | COACH; COACH; COACHEE | 200 each; S1's `participants` is `[{id: COACH, role: coach, is_member: true, names, email}, {id: COACHEE, role: coachee, ...}]`, S3's second participant is REMOVED with `is_member` false; without `include`, no `participants`; on `/nearest`, S1's `participants` are both `colleague`, COACH first. No session carries `coach` or `coachee`, and no `relationship` carries `coach_is_member` or `coachee_is_member` | `entity_api::coaching_session::EnrichedSession`; `entity_api::coaching_relationship::Participant::new` | Same participant shape as KIND-48 |
| KIND-50 | GET /users/{COACH_ID}/coaching-relationships with `role=coach`, `coachee`, `colleague`, `all` and none, R3 a colleague relationship; GET /users/{COACHEE_ID}/coaching-relationships with `role=colleague` and `coachee`, R1 a colleague relationship | COACH; COACHEE | 200 each; COACH: R1; empty; R3; R1 R3; R1 R3. COACHEE: R1; R2 | `entity_api::coaching_relationship::find_by_user_id_with_user_names` (`holding_role`, from each kind's rules) | A relationship matches a role when the seat the user holds has that role for its kind, so `coach` and `coachee` match coaching relationships only. Make R3 and R1 coaching again afterwards |

KIND-09's checks:

```sql
SELECT kind, count(*) FROM refactor_platform.coaching_relationships GROUP BY kind;

SELECT column_name, is_nullable, column_default FROM information_schema.columns
WHERE table_schema = 'refactor_platform' AND table_name = 'coaching_relationships'
  AND column_name = 'kind';

SELECT typowner::regrole FROM pg_type WHERE typname = 'coaching_relationship_kind';
```

## 5. Background jobs and emails

| ID | Trigger | Recipients today | Decided at (file:line) | Notes |
|---|---|---|---|---|
| JOB-01 | Session reminder sweep (`domain/src/jobs/session_reminder.rs:113`) | COACHEE only (the relationship's `coachee_id`), when still an org member and the session is in the window; coach never. Notice is measured from COACHEE's own notice row; a session with no row is reminded | `entity_api/src/coaching_session.rs:466`, `:473`, `:487`; `domain/src/emails.rs:1363` | Automated: SQL-shape mock only; check recipients by hand. |
| JOB-02 | Pending reschedule settle (`domain/src/jobs/coaching_session_pending_reschedule.rs:122`), queued by a PUT that moves the schedule (`domain/src/coaching_session.rs:405`) | Participants still org members (MAIL-02); each participant's notice row is stamped when their own email is written to the outbox (SCHED-10 to SCHED-12) | `domain/src/coaching_session.rs:778`; `domain/src/jobs/coaching_session_pending_reschedule.rs:187` | |
| JOB-03 | Transcript suggestions sweep (`domain/src/jobs/transcript_suggestions.rs:74`) | No email. SSE to participants still members (SSE-05). Prompt names coach and coachee | `domain/src/transcript_suggestion/extract.rs:240`, `:115` | |
| JOB-04 | Note image purge (`domain/src/jobs/coaching_session_image_purge.rs`) | None | not relationship-scoped | |
| JOB-05 | Password reset token cleanup (`domain/src/jobs/password_reset.rs`) | None | not relationship-scoped | |
| JOB-06 | Session reminder sweep (JOB-01) on a colleague relationship, both colleagues O1 members, no notice rows | Both colleagues, one claim and one email each per start; a second sweep sends nothing. A coaching relationship still reminds only its coachee | `entity_api/src/coaching_session.rs:447` (`Rules::is_reminded` per seat); `domain/src/emails.rs:1386` | Automated: SQL-shape mock and the sweep with mock Resend; check recipients by hand on Postgres. |
| JOB-07 | Pending reschedule settle (JOB-02) on an RC session MEMBER moved | COACH and MEMBER, both written to the outbox in one run, which settles the change | `domain/src/coaching_session.rs:778` | The outbox retries a refused copy; the change is not retried |
| MAIL-01 | Session scheduled: `POST /coaching_sessions` (`domain/src/coaching_session.rs:149`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:1056` | |
| MAIL-02 | Session rescheduled (JOB-02) | COACH and COACHEE, each only if still an org member; none if both left | `domain/src/coaching_session.rs:778` | |
| MAIL-03 | Session cancelled: `DELETE /coaching_sessions/{id}` (`domain/src/coaching_session.rs:541`); also orphan cancels in series edits (`domain/src/emails.rs:2640`) | COACH and COACHEE, members or not [membership]; nothing for a past session. In a series edit a removed participant gets only the cancel, not the reschedule | `domain/src/emails.rs:1612`, `:2623`, `:2640` | |
| MAIL-04 | Session invite resend: SES-24 (`domain/src/coaching_session.rs:597`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2135` | |
| MAIL-05 | Series scheduled: SER-01 (`domain/src/coaching_session_series.rs:145`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2224`, `:2041` | |
| MAIL-06 | Series rescheduled: SER-08 (`domain/src/coaching_session_series.rs:308`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2278` | |
| MAIL-07 | Series rescheduled from an occurrence on: SER-10 (`domain/src/coaching_session_series.rs:754`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2366` | |
| MAIL-08 | Series cancelled: SER-12 (`domain/src/coaching_session_series.rs`, `delete_with_future_sessions`) | COACH and COACHEE, members or not [membership]; nothing when no session is upcoming | `domain/src/emails.rs` (`SeriesNotice::cancellation`, `InviteParties::everyone`) | Written to the outbox in the delete's transaction |
| MAIL-09 | Series invite resend: SER-14 (`domain/src/coaching_session_series.rs:932`) | COACH and COACHEE, each only if still an org member [membership] | `domain/src/emails.rs:2099` | |
| MAIL-10 | Session reminder (JOB-01) | COACHEE only | `domain/src/emails.rs:1363`, `:1367` | Automated: SQL-shape mock only; check recipients by hand. |
| MAIL-11 | Action assigned: ACT-28, ACT-29, TRN-11, TRN-13 | Every listed assignee (the coach or coachee, per ACT rules) who is still an org member [membership] | `domain/src/emails.rs:2873`; `web/src/controller/action_controller.rs:136`, `:199`; `domain/src/transcript_suggestion/review.rs:380` | |
| MAIL-12 | Welcome (magic link): ORG-01 and `POST /organizations/{id}/users/{user}/resend-invite` | The new user; `inviter_first_name` / `inviter_full_name` name whoever sent the invite, and `coach_first_name` / `coach_full_name` are not sent | `domain/src/emails.rs:249`, `:297` | The variables name the inviter, who need not be the new user's coach (an admin inviting someone else's coachee) and won't be one in a phase 4 colleague invite |
| MAIL-13 | Added to organization: ORG-03 | The attached user | `domain/src/emails.rs:319` | |
| MAIL-14 | Password reset: `POST /password-reset/request` | The account owner | `domain/src/password_reset.rs:236`; not relationship-related | |
| MAIL-15 | Session reminder to a colleague (JOB-06) | Each colleague still an org member, once per start; `other_user_first_name` and `other_user_last_name` name the other colleague | `domain/src/emails.rs:1370`, `:1386` | Automated: the sweep with mock Resend; check recipients by hand. |
| MAIL-16 | Welcome to a colleague invitee: KIND-15 with a mock Resend | The new user; `inviter_first_name` / `inviter_full_name` name whoever sent the invite; `coach_first_name` / `coach_full_name` are not sent | `domain/src/emails.rs:307` | |
| MAIL-17 | Session reminder naming the other participant (JOB-01, JOB-06) | Each colleague's reminder carries `other_user_first_name`, `other_user_last_name` and `other_user_role` `colleague` naming the other colleague; a coachee's carries `other_user_role` `coach`; `coach_first_name` / `coach_full_name` are not sent | `domain/src/emails.rs:1409`, `:1512` | |
| MAIL-18 | Colleague session scheduled, rescheduled and cancelled, single and series (MAIL-01 to MAIL-03, MAIL-05 to MAIL-08 on RC) | Each colleague's email carries `other_user_role` `colleague` naming the other colleague | `domain/src/emails.rs:939` (`Participants::copies`) | No code change: the role comes from the relationship's rules |
| MAIL-19 | `.ics` DESCRIPTION of any invite: MAIL-01, MAIL-02, MAIL-04 to MAIL-07, MAIL-09 | Coaching: `Coach: {coach name}`, unchanged. Colleagues (RC): `Colleagues: {coach-seat name} and {coachee-seat name}`, no `Coach:` line | `domain/src/emails.rs:929` (`Participants::description_line`); `domain/src/gateway/ical.rs:331` | Both participants receive the same `.ics` |

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
| REL-22 | `web/src/controller/user/coaching_relationships_controller_sqlite_tests.rs::a_super_admin_lists_their_own_relationship_in_an_org_they_hold_no_role_in` |
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
| SES-43 | `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_removed_coachee_reads_no_nearest_session`; `domain/src/coaching_session_sqlite_tests.rs::a_users_own_nearest_sessions_skip_an_org_they_left` |
| SES-44 | `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_super_admin_lists_their_own_session_in_an_org_they_hold_no_role_in` |
| SES-45 | `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_super_admin_reads_their_nearest_session_in_an_org_they_hold_no_role_in` |
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
| SER-12 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::the_coach_deletes_a_series_with_no_upcoming_sessions`; with upcoming sessions, `domain/src/emails_sqlite_tests.rs::deleting_a_series_writes_both_cancellations_with_the_delete` |
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
| AGR-12 | `web/src/controller/agreement_controller_sqlite_tests.rs::signed_out_callers_cannot_reach_agreements` |
| ACT-01 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_coach_lists_actions_per_relationship`, `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::the_batch_list_is_keyed_actions_by_relationship` |
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
| ACT-30 | `web/src/controller/action_controller_sqlite_tests.rs::participants_update_the_action_status`; SQL shape (the enum cast) in `entity_api/src/action.rs::tests::update_status_writes_the_sent_status_with_the_enum_cast` |
| ACT-31 | `web/src/controller/action_controller_sqlite_tests.rs::participants_delete_the_action` |
| ACT-32 | `web/src/controller/action_controller_sqlite_tests.rs::non_participants_cannot_read_the_action` |
| ACT-33 | `web/src/controller/action_controller_sqlite_tests.rs::non_participants_cannot_create_an_action_in_the_session` |
| ACT-34 | `web/src/controller/action_controller_sqlite_tests.rs::non_participants_cannot_change_the_action` |
| ACT-35 | `web/src/controller/action_controller_sqlite_tests.rs::a_removed_coachee_cannot_reach_their_sessions_actions` |
| ACT-36 | `web/src/controller/action_controller_sqlite_tests.rs::assignees_outside_the_relationship_are_refused`, `web/src/controller/action_controller_sqlite_tests.rs::an_assignee_outside_the_relationship_is_told_to_name_its_participants` |
| ACT-37 | `web/src/controller/action_controller_sqlite_tests.rs::assigning_a_coachee_who_left_is_refused` |
| ACT-38 | `web/src/controller/action_controller_sqlite_tests.rs::a_coach_edits_an_action_still_assigned_to_a_coachee_who_left` |
| ACT-39 | `web/src/controller/action_controller_sqlite_tests.rs::adding_a_coachee_who_left_as_an_assignee_is_refused` |
| ACT-40 | `web/src/controller/action_controller_sqlite_tests.rs::a_goal_from_another_relationship_is_refused` |
| ACT-41 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_coachee_who_also_coaches_lists_each_relationship_by_their_role_in_it` |
| ACT-42 | `web/src/controller/action_controller_sqlite_tests.rs::signed_out_callers_cannot_reach_actions` |
| ACT-43 | `web/src/controller/user/action_controller_sqlite_tests.rs::a_super_admin_lists_their_own_actions_in_an_org_they_hold_no_role_in` |
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
| GOAL-29 | `web/src/controller/goal_controller_sqlite_tests.rs::participants_update_the_goal_status`; SQL shape (the enum cast) in `entity_api/src/goal.rs::tests::update_status_writes_the_sent_status_with_the_enum_cast` |
| GOAL-30 | `web/src/controller/goal_controller_sqlite_tests.rs::participants_read_the_goals_progress` |
| GOAL-31 | `web/src/controller/goal_controller_sqlite_tests.rs::non_participants_cannot_set_the_goal_status_or_read_its_progress` |
| GOAL-32 | `web/src/controller/goal_controller_sqlite_tests.rs::a_coach_creates_a_goal_from_a_session_of_the_relationship` |
| GOAL-33 | `web/src/controller/goal_controller_sqlite_tests.rs::signed_out_callers_cannot_reach_goals` |
| GOAL-34 | `web/src/controller/user/goal_controller_sqlite_tests.rs::a_removed_coachee_lists_none_of_their_own_goals`; `domain/src/goal_sqlite_tests.rs::a_users_own_goals_skip_an_org_they_left` |
| GOAL-35 | `web/src/controller/user/goal_controller_sqlite_tests.rs::a_super_admin_lists_their_own_goal_in_an_org_they_hold_no_role_in` |
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
| TOP-22 | `web/src/controller/coaching_session/topic_controller_sqlite_tests.rs::signed_out_callers_cannot_reach_topics` |
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
| TRN-19 | `web/src/controller/coaching_session/meeting_recording_controller_sqlite_tests.rs::either_participant_starts_a_recording_bot` (a stand-in bot provider; Recall itself is not called); the colleague 409 in `::a_colleagues_start_is_refused_while_the_others_bot_is_active` |
| TRN-20 | `web/src/controller/coaching_session/meeting_recording_controller_sqlite_tests.rs::either_participant_stops_the_recording_bot` (a stand-in bot provider; Recall itself is not called) |
| TRN-21 | `web/src/controller/coaching_session/meeting_recording_controller_sqlite_tests.rs::non_participants_cannot_reach_the_sessions_recording` |
| TRN-22 | `web/src/controller/coaching_session/meeting_recording_controller_sqlite_tests.rs::a_removed_coachee_cannot_reach_their_sessions_recording` |
| TRN-23 | `web/src/controller/coaching_session/image_controller_sqlite_tests.rs::participants_upload_a_note_image` |
| TRN-24 | `web/src/controller/coaching_session/image_controller_sqlite_tests.rs::non_participants_cannot_upload_a_note_image` |
| TRN-25 | `web/src/controller/coaching_session/image_controller_sqlite_tests.rs::participants_read_a_note_image` |
| TRN-26 | `web/src/controller/coaching_session/image_controller_sqlite_tests.rs::either_participant_deletes_and_restores_a_note_image` |
| TRN-27 | `web/src/controller/coaching_session/image_controller_sqlite_tests.rs::non_participants_cannot_read_delete_or_restore_a_note_image` |
| TRN-28 | `web/src/controller/coaching_session/transcript_suggestion_controller_sqlite_tests.rs::accepting_for_a_coachee_who_left_is_refused` |
| TRN-29 | `web/src/controller/coaching_session/transcription_controller_sqlite_tests.rs::signed_out_callers_cannot_reach_transcripts_suggestions_recordings_or_images` |
| TRN-34 | by hand against Postgres; the same race on SQLite in `web/src/controller/coaching_session/meeting_recording_controller_sqlite_tests.rs::a_start_during_dispatch_sees_the_claim_and_sends_no_second_bot` |
| TRN-35 | `web/src/controller/coaching_session/meeting_recording_controller_sqlite_tests.rs::a_start_in_an_archived_organization_is_refused`, `web/src/controller/coaching_session/meeting_recording_controller_sqlite_tests.rs::a_start_succeeds_once_the_organization_is_unarchived` |
| MEET-01 | by hand: first-read hydration takes `pg_advisory_xact_lock` and creates the link with the provider |
| MEET-02 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::only_the_coachs_create_attaches_a_meeting_link` (a reused R1 link; a new provider link needs Google or Zoom) |
| MEET-03 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_sets_the_sessions_meeting_link` |
| MEET-04 | `web/src/controller/coaching_session/meeting_recording_controller_sqlite_tests.rs::either_participant_starts_a_recording_bot` |
| MEET-05 | `web/src/controller/oauth_controller_sqlite_tests.rs::every_caller_reaches_only_their_own_connections` |
| MEET-06 | `web/src/controller/oauth_controller_sqlite_tests.rs::signed_out_callers_cannot_reach_connections` |
| SRCH-01 | by hand: full-text search |
| SRCH-02 | by hand: full-text search |
| SRCH-03 | by hand: full-text search |
| SRCH-04 | `web/src/controller/search_controller_sqlite_tests.rs::members_outside_every_relationship_find_nothing` |
| SRCH-05 | by hand: full-text search |
| SRCH-06 | `web/src/controller/search_controller_sqlite_tests.rs::a_removed_coachee_finds_nothing` |
| SRCH-07 | by hand: full-text search; the relationships the filters leave in `domain/src/search_sqlite_tests.rs::a_members_filters_narrow_but_never_widen_what_they_search` |
| SRCH-08 | `web/src/controller/search_controller_sqlite_tests.rs::a_relationship_filter_never_widens_a_members_search` |
| SRCH-09 | by hand: full-text search; the relationships the filters leave in `domain/src/search_sqlite_tests.rs::a_super_admins_filters_narrow_an_otherwise_unrestricted_search` |
| SRCH-10 | by hand: full-text search |
| SRCH-11 | `web/src/controller/search_controller_sqlite_tests.rs::signed_out_callers_cannot_search` |
| JWT-01 | `web/src/controller/jwt_controller_sqlite_tests.rs::participants_get_a_token_for_the_sessions_document` |
| JWT-02 | `web/src/controller/jwt_controller_sqlite_tests.rs::non_participants_get_no_token` |
| JWT-03 | `web/src/controller/jwt_controller_sqlite_tests.rs::a_removed_coachee_gets_no_token` |
| JWT-04 | `web/src/controller/jwt_controller_sqlite_tests.rs::signed_out_callers_get_no_token` |
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
| ORG-29 | `web/src/controller/organization_controller_sqlite_tests.rs::a_super_admin_renames_an_organization` |
| ORG-30 | `web/src/controller/organization_controller_sqlite_tests.rs::only_a_super_admin_renames_an_organization` |
| ORG-31 | `web/src/controller/organization_controller_sqlite_tests.rs::a_super_admin_deletes_an_empty_organization` |
| ORG-32 | `web/src/controller/organization_controller_sqlite_tests.rs::only_a_super_admin_deletes_an_organization` |
| ORG-33 | `web/src/controller/organization_controller_sqlite_tests.rs::a_super_admin_archives_an_organization` |
| ORG-34 | `web/src/controller/organization_controller_sqlite_tests.rs::only_a_super_admin_archives_an_organization` |
| ORG-35 | `web/src/controller/organization_controller_sqlite_tests.rs::a_super_admin_unarchives_an_organization` |
| ORG-36 | `web/src/controller/organization_controller_sqlite_tests.rs::only_a_super_admin_unarchives_an_organization` |
| ORG-37 | `web/src/controller/organization_controller_sqlite_tests.rs::members_and_super_admins_read_an_organization` |
| ORG-38 | `web/src/controller/organization_controller_sqlite_tests.rs::non_members_cannot_read_an_organization` |
| ORG-39 | `web/src/controller/organization_controller_sqlite_tests.rs::an_unknown_organization_is_not_found` |
| SSE-01 | `web/src/sse/handler_sqlite_tests.rs::every_caller_opens_an_event_stream` |
| SSE-02 | `web/src/sse/handler_sqlite_tests.rs::both_participants_hear_of_an_action_assigned_to_the_coach` |
| SSE-03 | `web/src/sse/handler_sqlite_tests.rs::a_removed_coachee_hears_nothing_of_their_relationship` |
| SSE-04 | `web/src/sse/handler_sqlite_tests.rs::both_participants_hear_of_a_goal_change` |
| SSE-05 | `web/src/sse/handler_sqlite_tests.rs::both_participants_hear_that_suggestions_changed` (through a scan start, which shares the sweep's `extract::notify`); `domain/src/transcript_suggestion/extract_sqlite_tests.rs::every_completed_run_announces_once_with_or_without_suggestions` |
| SSE-06 | `web/src/sse/handler_sqlite_tests.rs::both_participants_hear_of_a_recording_update` |
| SSE-07 | `web/src/sse/handler_sqlite_tests.rs::signed_out_callers_cannot_open_an_event_stream` |
| SCHED-01 | `domain/src/coaching_session_sqlite_tests.rs::create_records_the_acting_user_as_scheduler`, `domain/src/coaching_session_sqlite_tests.rs::create_stamps_both_participants_notice_at_the_sessions_creation` |
| SCHED-02 | `domain/src/coaching_session_sqlite_tests.rs::create_records_the_acting_user_as_scheduler` (a caller's value is overwritten); by hand for the request body and response, which `#[serde(skip)]` decides |
| SCHED-03 | `domain/src/coaching_session_sqlite_tests.rs::create_reusing_a_meeting_link_with_no_recorded_creator_records_the_coach`, `domain/src/coaching_session_sqlite_tests.rs::create_reusing_a_meeting_link_keeps_its_original_creator`, `domain/src/coaching_session_sqlite_tests.rs::create_without_a_provider_records_no_link_creator`; by hand for a new link: it calls the provider |
| SCHED-04 | `domain/src/coaching_session_update_sqlite_tests.rs::moving_a_session_records_the_acting_user_as_scheduler` |
| SCHED-05 | `domain/src/coaching_session_update_sqlite_tests.rs::a_title_edit_leaves_the_scheduler_as_it_was` |
| SCHED-06 | `domain/src/coaching_session_series_sqlite_tests.rs::a_created_series_records_the_acting_user_on_every_session`, `domain/src/coaching_session_series_sqlite_tests.rs::a_created_series_stamps_both_participants_notice_at_each_sessions_creation` |
| SCHED-07 | `domain/src/coaching_session_series_sqlite_tests.rs::a_rescheduled_series_records_the_acting_user_on_the_sessions_it_creates`; by hand end to end: the series update takes Postgres advisory locks |
| SCHED-08 | `domain/src/coaching_session_series_sqlite_tests.rs::move_to_slot_records_who_moved_the_session`, `domain/src/coaching_session_series_sqlite_tests.rs::a_following_reschedule_restamps_only_the_participants_it_reached`; by hand end to end: reschedule takes Postgres advisory locks |
| SCHED-09 | `domain/src/coaching_session_sqlite_tests.rs::mark_hydrated_stores_the_link_creator_and_keeps_the_scheduler` |
| SCHED-10 | `domain/src/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_due_move_is_written_to_both_members_their_notice_stamped_and_settled` |
| SCHED-11 | `domain/src/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_member_who_left_is_not_written_to_and_the_change_settles` |
| SCHED-12 | `domain/src/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_failed_write_is_retried_then_given_up` |
| SCHED-13 | by hand: the OpenAPI document is read from a running server |
| SCHED-14 | by hand: the migrations run on Postgres |
| SCHED-15 | `domain/src/coaching_session_update_sqlite_tests.rs::setting_a_meeting_link_records_the_acting_user_as_its_creator`, `domain/src/coaching_session_update_sqlite_tests.rs::a_title_edit_leaves_the_link_creator_as_it_was` |
| SCHED-16 | `domain/src/coaching_session_sqlite_tests.rs::create_with_a_pasted_meeting_link_records_the_acting_user_as_its_creator` |
| JOB-01 | by hand: raw Postgres SQL; SQL shape in `entity_api/src/coaching_session.rs::tests::claim_due_reminders_skips_former_organization_members`, `entity_api/src/coaching_session.rs::tests::claim_due_reminders_membership_mirrors_is_member`, `entity_api/src/coaching_session.rs::tests::claim_due_reminders_skips_sessions_booked_on_short_notice` |
| JOB-02 | `domain/src/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_due_move_is_written_to_both_members_their_notice_stamped_and_settled`, `domain/src/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_member_who_left_is_not_written_to_and_the_change_settles` |
| JOB-03 | `domain/src/transcript_suggestion/prompt_tests.rs::render_writes_the_session_existing_actions_and_transcript`; events as SSE-05 |
| JOB-04 | `domain/src/jobs/coaching_session_image_purge_sqlite_tests.rs` (no recipients) |
| JOB-05 | `domain/src/password_reset.rs::tests::sweep_old_attempts_returns_deleted_count` (no recipients) |
| MAIL-01 | `domain/src/emails_sqlite_tests.rs::members_only_emails_reach_only_participants_holding_a_role`, `domain/src/emails_sqlite_tests.rs::session_scheduled_with_the_coachee_holding_no_role_reaches_only_the_coach`, `domain/src/emails_sqlite_tests.rs::session_create_with_the_coach_holding_no_role_reaches_only_the_coachee` |
| MAIL-02 | `domain/src/emails_tests.rs::test_session_rescheduled_envelopes`; `domain/src/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_member_who_left_is_not_written_to_and_the_change_settles` |
| MAIL-03 | `domain/src/emails_sqlite_tests.rs::single_cancel_with_the_coachee_holding_no_role_reaches_both`, `domain/src/emails_sqlite_tests.rs::single_cancel_with_the_coach_holding_no_role_reaches_both`, `domain/src/emails_sqlite_tests.rs::a_series_edit_cancels_an_orphan_for_a_coachee_holding_no_role` |
| MAIL-04 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_session_resend_mails_both_members`, `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_coach_who_left_cannot_resend_a_sessions_invite`; `domain/src/emails_sqlite_tests.rs::session_resend_with_the_coach_holding_no_role_reaches_only_the_coachee`; `web/src/controller/resend_invite_tests.rs::session_resend_bumps_a_one_off`, `web/src/controller/resend_invite_tests.rs::resend_with_a_removed_coach_mails_only_the_coachee`, `web/src/controller/resend_invite_tests.rs::resend_to_a_removed_coachee_is_refused` |
| MAIL-05 | `domain/src/emails_sqlite_tests.rs::members_only_emails_reach_only_participants_holding_a_role`, `domain/src/emails_sqlite_tests.rs::series_create_with_the_coach_holding_no_role_reaches_only_the_coachee` |
| MAIL-06 | `domain/src/emails_sqlite_tests.rs::members_only_emails_reach_only_participants_holding_a_role`; `domain/src/coaching_session_series.rs::tests::m5_09_series_reschedule_emails_only_current_members` |
| MAIL-07 | `domain/src/emails_sqlite_tests.rs::members_only_emails_reach_only_participants_holding_a_role`, `domain/src/emails_sqlite_tests.rs::a_following_reschedule_reports_a_coachee_holding_no_role_as_not_a_member`; `domain/src/coaching_session_series.rs::tests::m5_10_reschedule_following_emails_only_current_members` |
| MAIL-08 | `domain/src/emails_sqlite_tests.rs::deleting_a_series_writes_both_cancellations_with_the_delete`, `domain/src/emails_sqlite_tests.rs::deleting_a_series_with_nothing_upcoming_writes_no_cancellation` |
| MAIL-09 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::a_series_resend_mails_both_members`, `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::a_coach_who_left_cannot_resend_a_series_invite`; `domain/src/emails_sqlite_tests.rs::series_resend_with_the_coach_holding_no_role_reaches_only_the_coachee`; `web/src/controller/resend_invite_tests.rs::series_resend_bumps_the_series_and_raises_its_members`, `web/src/controller/resend_invite_tests.rs::resend_with_a_removed_coach_mails_only_the_coachee`, `web/src/controller/resend_invite_tests.rs::resend_to_a_removed_coachee_is_refused` |
| MAIL-10 | `domain/src/emails_tests.rs::test_send_session_reminder_email_goes_only_to_the_coachee`; `domain/src/emails_sqlite_tests.rs::a_reminder_recipient_is_mailed_only_while_holding_a_role` |
| MAIL-11 | `domain/src/emails_sqlite_tests.rs::action_assigned_reaches_only_an_assignee_holding_a_role` |
| MAIL-12 | `domain/src/emails_tests.rs::test_send_welcome_email_success` |
| MAIL-13 | `domain/src/emails_added_to_organization_tests.rs::send_added_to_organization_email_wire_contract` |
| MAIL-14 | `domain/src/emails_tests.rs::test_send_password_reset_email_wire_contract` |
| KIND-04 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::an_admin_creates_a_colleague_relationship` |
| KIND-11 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_colleague_create_returns_the_pairs_colleague_relationship_either_way_round` |
| KIND-12 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_colleague_create_is_refused_for_a_coaching_pair` |
| KIND-13 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_coaching_create_is_refused_for_a_colleague_pair` |
| KIND-14 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_coaching_pair_may_also_coach_the_other_way_round` |
| KIND-15 | `web/src/controller/organization/user_controller_sqlite_tests.rs::admins_add_a_member_as_a_colleague` |
| KIND-16 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::admins_attach_a_member_as_a_colleague` |
| KIND-17 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::a_refused_relationship_grants_no_role` |
| KIND-18 | `web/src/controller/organization/user_controller_sqlite_tests.rs::a_refused_relationship_adds_no_member_and_sends_no_email` |
| JOB-06 | by hand: raw Postgres SQL; SQL shape in `entity_api/src/coaching_session.rs::tests::claim_due_reminders_also_reminds_the_coach_seat_of_a_colleague_relationship`, `entity_api/src/coaching_session.rs::tests::claim_due_reminders_candidates_mirror_is_reminded`; the sweep in `domain/src/jobs/session_reminder_tests.rs::both_colleagues_are_reminded_once_each_naming_the_other`, `domain/src/jobs/session_reminder_tests.rs::a_coach_is_not_reminded_of_a_coaching_session` |
| MAIL-15 | `domain/src/jobs/session_reminder_tests.rs::both_colleagues_are_reminded_once_each_naming_the_other` |
| SES-46 | `web/src/controller/coaching_session_controller_sqlite_tests.rs::a_colleague_cannot_schedule_with_a_colleague_who_left` |
| SER-24 | `web/src/controller/coaching_session_series_controller_sqlite_tests.rs::a_series_with_a_colleague_who_left_is_refused` |
| TRN-30 | replaced by TRN-33 |
| TRN-31 | `domain/src/transcript_suggestion/review_sqlite_tests.rs::a_colleague_session_can_be_scanned` |
| SER-25 | `domain/src/coaching_session_series_sqlite_tests.rs::a_colleague_series_follows_the_zone_of_the_colleague_who_created_it` |
| SER-26 | `domain/src/coaching_session_series_sqlite_tests.rs::a_rescheduled_colleague_series_follows_the_zone_of_the_colleague_who_moved_it`, `domain/src/coaching_session_series_sqlite_tests.rs::a_rescheduled_coaching_series_keeps_the_coachs_zone`; by hand for reschedule-following: it takes Postgres advisory locks |
| SCHED-17 | `domain/src/coaching_session_sqlite_tests.rs::a_colleague_session_takes_the_schedulers_default_duration` |
| SCHED-18 | `domain/src/coaching_session_sqlite_tests.rs::a_colleague_session_links_with_the_other_colleagues_account_when_the_scheduler_has_none`, `domain/src/coaching_session_sqlite_tests.rs::a_colleague_session_gets_no_link_when_neither_colleague_is_connected` |
| SCHED-19 | `domain/src/coaching_session_sqlite_tests.rs::a_coaching_session_never_links_with_the_coachees_account` |
| SCHED-20 | `domain/src/emails_tests.rs::a_colleague_sessions_invite_follows_its_schedulers_zone`, `domain/src/emails_tests.rs::a_coaching_sessions_invite_follows_the_coachs_zone_whoever_scheduled_it` |
| SCHED-21 | `domain/src/coaching_session.rs::tests::hydration_links_with_the_remaining_colleagues_account_when_the_scheduler_left`, `domain/src/coaching_session.rs::tests::hydration_makes_no_link_when_no_remaining_colleague_is_connected`, `domain/src/coaching_session.rs::tests::hydration_links_with_the_coachs_account_only_while_the_coach_is_a_member` |
| SCHED-22 | `domain/src/coaching_session_sqlite_tests.rs::a_colleague_session_with_no_provider_links_with_the_other_colleagues_connection`, `domain/src/coaching_session_sqlite_tests.rs::a_connected_colleague_who_asks_for_no_provider_gets_no_link`, `domain/src/coaching_session_sqlite_tests.rs::a_coaching_session_with_no_provider_never_links_with_the_coachees_connection` |
| ACT-44 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::colleagues_cannot_scope_relationship_actions_to_a_seat` |
| ACT-45 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::colleagues_list_relationship_actions_whole_or_by_user` |
| ACT-46 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_seat_scoped_batch_leaves_out_colleague_relationships` |
| ACT-47 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_colleague_in_no_coaching_relationship_gets_an_empty_seat_scoped_batch` |
| ACT-48 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_coachee_who_is_also_a_colleague_cannot_scope_the_batch_to_the_coach` |
| ACT-49 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_colleague_scoped_batch_lists_only_the_other_colleagues_actions` |
| ACT-50 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_colleague_scoped_batch_is_empty_without_a_colleague_relationship` |
| ACT-51 | `web/src/controller/organization/coaching_relationship/actions_controller_sqlite_tests.rs::a_colleague_scopes_relationship_actions_to_the_other_colleague`; `entity_api/src/action.rs::tests::a_role_scope_names_the_holder_who_is_not_the_caller_or_no_one`, `entity_api/src/action.rs::tests::role_scopes_name_each_kinds_participants_as_pinned` |
| GOAL-36 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::colleagues_cannot_scope_goal_progress_to_a_seat` |
| KIND-19 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_coaching_relationship_tells_each_participant_what_they_may_do` |
| KIND-20 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_colleague_relationship_tells_each_colleague_they_may_do_everything` |
| KIND-21 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::an_admin_is_no_viewer_of_the_relationships_they_list` |
| KIND-22 | `entity/src/coaching_relationships_access_tests.rs::a_viewer_answers_what_the_enforcing_functions_answer`, `entity/src/coaching_relationships_access_tests.rs::a_coach_who_left_may_not_schedule`; no route returns a relationship to a participant who left |
| KIND-23 | `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::an_included_relationship_tells_the_caller_what_they_may_do`, `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_colleague_sessions_nearest_read_lists_both_as_colleagues_coach_seat_first` (`/nearest`) |
| KIND-24 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::scheduling_is_blocked_by_the_participant_who_left`; `domain/src/coaching_relationship.rs::tests::scheduling_blocked_by_names_whom_the_scheduling_conflict_names` |
| KIND-25 | `web/src/controller/user/coaching_relationships_controller_sqlite_tests.rs::a_coachee_is_told_what_they_may_do_in_each_listed_relationship` |
| KIND-26 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_created_relationship_names_no_viewer_for_the_admin` |
| MAIL-16 | `domain/src/emails_tests.rs::test_send_welcome_email_success`, `domain/src/emails_tests.rs::test_send_welcome_email_escapes_name_with_specials` |
| MAIL-17 | `domain/src/jobs/session_reminder_tests.rs::each_colleagues_reminder_names_the_other_a_colleague`, `domain/src/jobs/session_reminder_tests.rs::a_coachees_reminder_names_their_coach`; `domain/src/emails_tests.rs::test_send_session_reminder_email_variables` |
| MAIL-18 | `domain/src/emails_tests.rs::each_colleagues_scheduled_email_names_the_other_a_colleague`, `domain/src/emails_tests.rs::each_colleagues_rescheduled_email_names_the_other_a_colleague`, `domain/src/emails_tests.rs::each_colleagues_cancelled_email_names_the_other_a_colleague`; series sends share `Participants::copies` |
| MAIL-19 | `domain/src/emails_tests.rs::a_coaching_description_names_the_coach_as_before`, `domain/src/emails_tests.rs::a_colleague_description_names_both_colleagues_and_no_coach`, `domain/src/emails_tests.rs::each_colleagues_scheduled_email_names_the_other_a_colleague` |
| SES-47 | `domain/src/emails_sqlite_tests.rs::a_colleague_resend_the_other_colleague_missed_fails_and_stays_unannounced`, `domain/src/emails_sqlite_tests.rs::a_colleague_resend_that_missed_only_the_scheduler_succeeds_and_is_announced` |
| JOB-07 | `domain/src/jobs/coaching_session_pending_reschedule_sqlite_tests.rs::a_due_move_is_written_to_both_members_their_notice_stamped_and_settled` |
| SER-27 | `domain/src/coaching_session_series_sqlite_tests.rs::a_colleague_following_reschedule_the_other_colleague_missed_stays_pending`, `domain/src/coaching_session_series_sqlite_tests.rs::a_colleague_following_reschedule_that_missed_only_the_scheduler_settles` |
| ORG-40 | `web/src/controller/organization/user_controller_sqlite_tests.rs::a_member_body_with_the_replaced_coach_id_is_refused` |
| ORG-41 | `web/src/controller/organization/user_controller_sqlite_tests.rs::admins_add_a_member_as_the_coach`; every kind and role in `domain/src/coaching_relationship.rs::tests::an_invitee_takes_the_seat_holding_their_requested_role` |
| ORG-42 | `web/src/controller/organization/user_controller_sqlite_tests.rs::a_role_the_kind_lacks_adds_no_member_and_sends_no_email`; `domain/src/coaching_relationship.rs::tests::an_invitee_takes_the_seat_holding_their_requested_role` |
| ORG-43 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::admins_attach_a_member_as_the_coach` |
| ORG-44 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::a_coachee_attached_as_their_coachs_coach_coaches_them_back`, `web/src/controller/organization/user/role_controller_sqlite_tests.rs::a_coachee_attached_as_their_coachs_colleague_is_refused` |
| SER-28 | `domain/src/coaching_session_series_sqlite_tests.rs::an_unchanged_colleague_series_saved_from_another_zone_changes_nothing` |
| TRN-32 | `domain/src/transcription_completion_tests.rs::attribution_matches_the_coach_and_reads_attendees_as_the_link_maker`, `domain/src/transcription_completion_tests.rs::attribution_without_a_recorded_link_maker_reads_as_the_coach` |
| TRN-33 | `domain/src/transcription_analysis_hook_sqlite_tests.rs::completing_a_colleague_transcript_writes_no_analysis`, `domain/src/transcription_analysis_hook_sqlite_tests.rs::completing_with_a_target_writes_one_pending_analysis_with_the_segments` |
| KIND-27 | `domain/src/coaching_relationship_sqlite_tests.rs::a_returned_colleague_relationship_names_each_seat_by_its_own_user` |
| SER-29 | `domain/src/coaching_session_series.rs::tests::an_unchanged_following_save_from_another_zone_moves_nothing` (mock: reschedule-following takes Postgres advisory locks) |
| REL-23 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_created_relationship_is_recorded` |
| REL-24 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_returned_relationship_is_not_recorded_again` |
| ORG-45 | `web/src/controller/organization/user_controller_sqlite_tests.rs::adding_a_member_records_the_membership_and_the_relationship` |
| ORG-46 | `web/src/controller/organization/user_controller_sqlite_tests.rs::a_refused_add_records_nothing` |
| ORG-47 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::attaching_a_member_records_the_membership_and_the_relationship` |
| ORG-48 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::attaching_a_member_with_their_existing_relationship_records_only_the_membership` |
| ORG-49 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::a_refused_attach_records_nothing` |
| ORG-50 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::removing_a_member_records_it` |
| ORG-51 | `web/src/controller/organization/user/role_controller_sqlite_tests.rs::changing_a_members_role_records_both_roles` |
| ORG-52 | `web/src/controller/organization/user_controller_sqlite_tests.rs::deleting_a_member_records_each_relationship_it_deletes` |
| ORG-53 | `web/src/controller/organization/user_controller_sqlite_tests.rs::deleting_a_member_records_their_removal`, `web/src/controller/organization/user_controller_sqlite_tests.rs::deleting_a_member_records_each_relationship_it_deletes` |
| KIND-28 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::an_unconfirmed_switch_to_colleague_lists_its_consequences_and_changes_nothing` |
| KIND-29 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_confirmed_switch_to_colleague_gives_the_former_coachee_everything` |
| KIND-30 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_confirmed_switch_to_coaching_seats_the_chosen_coach` |
| KIND-31 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_switch_naming_the_coach_wrongly_is_refused` |
| KIND-32 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::reversing_a_coaching_relationship_needs_confirmation_then_swaps_the_seats` |
| KIND-33 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_pair_coaching_each_other_cannot_switch_either_relationship` |
| KIND-34 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::only_the_organizations_admins_switch_a_relationship` |
| KIND-35 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::switching_to_what_the_relationship_already_is_changes_nothing` |
| KIND-36 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_relationship_whose_participant_left_cannot_switch` |
| KIND-37 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_switch_marks_the_newly_reminded_participants_imminent_reminders_sent`; by hand for the sweep's send: its claim is Postgres-only SQL |
| KIND-38 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_confirmed_switch_resends_every_upcoming_invite_to_both_once`; `domain/src/jobs/coaching_session_pending_reschedule.rs::tests::from_config_resends_invites_only_with_a_key_and_both_scheduled_templates` |
| KIND-39 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::switches_before_the_sweep_owe_each_invite_once` |
| KIND-40 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_participant_who_left_before_the_sweep_gets_no_invite` |
| KIND-41 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_past_session_is_owed_no_invite` |
| KIND-42 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::an_unconfirmed_or_refused_switch_owes_nothing_and_tells_no_one` |
| KIND-43 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_switch_tells_both_participants_once`; `sse/src/message.rs::tests::coaching_relationship_kind_changed_serializes_to_expected_wire_shape` |
| KIND-44 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_relationship_in_an_archived_organization_cannot_switch` |
| KIND-45 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_relationship_read_carries_each_participants_email`, `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::an_admins_list_carries_each_relationships_participants_emails`, `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_members_list_carries_each_relationships_participants_emails`, `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_confirmed_reversal_returns_each_swapped_seats_email`; `web/src/controller/user/coaching_relationships_controller_sqlite_tests.rs::a_users_list_carries_each_relationships_participants_emails`; `domain/src/coaching_relationship_sqlite_tests.rs::a_returned_colleague_relationship_carries_each_seats_own_email` |
| KIND-46 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_confirmed_switch_is_recorded_with_the_seats_after_it`, `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::an_unconfirmed_refused_or_unchanged_switch_records_nothing` |
| KIND-47 | `domain/src/emails_tests.rs::scheduled_emails_carry_the_kinds_session_label`, `domain/src/emails_tests.rs::rescheduled_emails_carry_the_kinds_session_label`, `domain/src/emails_tests.rs::cancelled_emails_carry_the_kinds_session_label`, `domain/src/emails_tests.rs::series_scheduled_emails_carry_the_kinds_session_label`, `domain/src/emails_tests.rs::series_rescheduled_emails_carry_the_kinds_session_label`, `domain/src/emails_tests.rs::series_cancelled_emails_carry_the_kinds_session_label`, `domain/src/emails_tests.rs::a_cancelled_sessions_invite_names_the_kinds_session`, `domain/src/emails_tests.rs::a_cancelled_series_invite_names_the_kinds_session`; `domain/src/jobs/session_reminder_tests.rs::each_reminder_carries_its_kinds_session_label` |
| KIND-48 | `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_coaching_relationship_lists_its_coach_then_its_coachee`, `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_colleague_relationship_lists_both_as_colleagues_coach_seat_first`, `web/src/controller/organization/coaching_relationship_controller_sqlite_tests.rs::a_coach_keeps_a_relationship_whose_coachee_left`; `entity_api/src/coaching_relationship.rs::wire_tests::schema_lists_exactly_the_serialized_keys` |
| KIND-49 | `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::an_included_relationship_lists_the_sessions_coach_then_its_coachee`, `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_coachs_session_list_marks_a_coachee_who_left`, `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_session_list_without_its_relationship_lists_no_participants`, `web/src/controller/user/coaching_session_controller_sqlite_tests.rs::a_colleague_sessions_nearest_read_lists_both_as_colleagues_coach_seat_first`; `entity_api/src/coaching_relationship.rs::wire_tests::a_relationship_with_membership_serializes_exactly_its_schemas_keys` |
| KIND-50 | `web/src/controller/user/coaching_relationships_controller_sqlite_tests.rs::a_coach_with_a_colleague_relationship_lists_each_under_its_own_role`, `web/src/controller/user/coaching_relationships_controller_sqlite_tests.rs::a_colleague_in_the_coachee_seat_lists_it_as_a_colleague` |
