# Test Plan: Manually Testing Remove-From-Organization Access Revocation

Verify that removing a member (coach or coachee) from an organization revokes
their access to that organization's coaching data, while destroying none of it
and leaving the other participant's access untouched.
Issue: [refactor-group/refactor-platform-rs#377](https://github.com/refactor-group/refactor-platform-rs/issues/377).

The single most important property: **removing is not deleting, and it is not
cosmetic.** Every session, note, action, agreement, topic and goal must survive
the removal, the still-member participant must still read all of it, and the
removed user must be denied all of it at the API, not merely have it hidden in
the UI.

> [!WARNING]
> These scenarios delete real membership rows and require a user who has
> accumulated real coaching history. Use a local stack with disposable data.

> [!IMPORTANT]
> The UI hiding an organization is **not** evidence of revocation. Before this
> change, dropping the membership row removed the organization from the switcher
> while leaving full API access intact. Scenario C is the scenario that actually
> proves the feature works, and it must be run with an API client, not the app.

## 1. Prerequisites

- Backend on branch `feat/remove-org-member-revokes-access`.
- An API client that can send an authenticated request (cookie jar), e.g. `curl -b`,
  Postman, or the browser devtools console on a logged-in tab.
- One organization (call it *Acme*) containing at least:
  - an **Admin** (the remover),
  - a **coach** (call them *Casey*),
  - a **coachee** (call them *Robin*),
  - a second organization (*Globex*) where Robin is also a member, to prove
    removal is scoped.
- A coaching relationship between Casey and Robin in Acme, carrying **real
  history**: at least one coaching session, and within it at least one note, one
  action, one agreement, and one topic. Under the previous behavior this history
  is exactly what made removal impossible, so it is the precondition that matters.

Record these IDs before starting, you will need them for the direct API calls:

| Thing | Where to find it |
|---|---|
| `organization_id` (Acme) | URL of the org page |
| `user_id` (Robin) | Members list, or the user lookup endpoint |
| `coaching_session_id` | URL of the coaching session page |
| `coaching_relationship_id` | Network tab on the sessions list request |

Role naming, as elsewhere in this codebase: the UI says **Member**, the JSON wire
value is `"User"`, and the database stores `user`. All three are the same role.

## 2. Scenario A: removal succeeds despite coaching history

This is the regression the feature exists to fix. Before this change the request
returned 409 `user_has_coaching_history`.

Logged in as the Acme Admin.

1. Open Acme → **Members**.
2. Remove Robin.

```
DELETE /organizations/{acme_id}/users/{robin_id}/role
```

| Check | Expected |
|---|---|
| HTTP status | **204 No Content** |
| Response body | empty |
| Members list after refresh | Robin gone |
| Server log | no foreign key error, no 409 |

If this returns 409, the history guard was not removed. If it returns 500 with a
foreign key violation, the relationship delete was not removed.

## 3. Scenario B: nothing was destroyed

Immediately after Scenario A, still as the Admin, and **before** logging in as
anyone else.

Query the database directly (or use the coach's view in Scenario D):

```sql
-- all of these must still return their pre-removal counts
SELECT count(*) FROM refactor_platform.coaching_relationships
  WHERE organization_id = '<acme_id>' AND coachee_id = '<robin_id>';
SELECT count(*) FROM refactor_platform.coaching_sessions
  WHERE coaching_relationship_id = '<relationship_id>';
SELECT count(*) FROM refactor_platform.notes    WHERE coaching_session_id = '<session_id>';
SELECT count(*) FROM refactor_platform.actions  WHERE coaching_session_id = '<session_id>';
SELECT count(*) FROM refactor_platform.agreements WHERE coaching_session_id = '<session_id>';
```

And exactly one row should have disappeared:

```sql
-- expect 0 rows
SELECT * FROM refactor_platform.user_roles
  WHERE user_id = '<robin_id>' AND organization_id = '<acme_id>';
```

| Check | Expected |
|---|---|
| relationship, sessions, notes, actions, agreements, topics, goals | **unchanged counts** |
| `user_roles` row for Robin in Acme | **gone** |
| Robin's `users` row | **still present** |

## 4. Scenario C: the removed user is denied at the API

**This is the scenario that proves the feature.** Do not substitute the UI for it.

Log in as **Robin**. The app will no longer show Acme in the organization
switcher, which is expected but proves nothing. Now hit the API directly with
Robin's session cookie, using the IDs recorded earlier.

| Request | Expected |
|---|---|
| `GET /coaching_sessions/{session_id}` | **403** |
| `GET /notes?coaching_session_id={session_id}` | **403** |
| `GET /actions?coaching_session_id={session_id}` | **403** |
| `GET /agreements?coaching_session_id={session_id}` | **403** |
| `GET /coaching_sessions/{session_id}/topics` | **403** |
| `GET /goals?coaching_relationship_id={relationship_id}` | **403** |
| `GET /organizations/{acme_id}/coaching_relationships/{relationship_id}` | **403** |
| `GET /jwt/generate_collab_token?coaching_session_id={session_id}` | **403** |

A **200 anywhere in this table is a failure of the feature**, even if the UI
looks correct. A 401 means the session expired, log in again and retry. A 404
is also a failure unless the resource genuinely does not exist, since these
paths are specified to return 403 for a non-member participant.

## 5. Scenario D: the coach is unaffected

Log in as **Casey**, who is still an Acme member and still the coach on the same
relationship. Using the same IDs:

| Request | Expected |
|---|---|
| every row from the Scenario C table | **200** |
| the coaching session page in the UI | renders, with notes and actions intact |
| the note body | **identical to before the removal** |

This is the half that a naive "delete the relationship" implementation breaks.
If Casey gets 403 or 404, access was revoked too broadly. If the note content is
empty or missing, data was destroyed.

## 6. Scenario E: removal is scoped to one organization

Still as Robin.

| Check | Expected |
|---|---|
| Organization switcher | **Globex still listed** |
| Globex coaching sessions, notes, actions | **all still 200** |
| Login itself | still works; the account was not deleted |

Removal from Acme must not touch Globex membership or any other organization.

## 7. Scenario F: removing a coach behaves the same way

Reset the data (or use a second relationship), then remove **Casey** instead of
Robin, using the same endpoint.

| Check | Expected |
|---|---|
| HTTP status | 204 |
| Casey's access to the Acme session, notes, actions | **403** |
| Robin's access to the same rows | **200**, provided Robin is still a member |
| The relationship row | still present |

Coach and coachee are treated identically. There is deliberately no guard on
removing a coach who still has coachees; those coachees keep their history and
the admin can reassign a coach afterwards.

## 8. Scenario G: the existing guards still hold

These were not changed by this work, and are here to catch a regression.

| Action | Expected |
|---|---|
| Admin removes **themselves** | **403** |
| Removing the organization's **only Admin** | **409**, membership intact |
| Removing a user who holds **no role** in the org | **404** |
| A **plain member** attempts any removal | **403** |
| A **SuperAdmin** removes a member of any org | **204** |
| An **Admin of a different org** attempts the removal | **403** |

## 9. Scenario H: SSE stops delivering to the removed user

Access revocation has to cover the live event stream, not just request/response.

1. Log in as Robin in one browser and open a page that holds the SSE connection.
   Confirm in devtools that `/sse` is open and streaming.
2. **Leave that tab open and connected.**
3. In another browser, as the Admin, remove Robin from Acme.
4. As Casey, create a new note and a new action on the shared Acme session.

| Check | Expected |
|---|---|
| Robin's open `/sse` stream | receives **no** event for the new note or action |
| Casey's stream | receives both events normally |

A note or action arriving on Robin's stream after removal means the participant
notify set is not membership-filtered, and content is still leaking to a removed
user over a connection that was authorized before the removal.

## 10. Known gaps (expected failures, not bugs)

These are deferred to separate issues. Record them if seen, do not treat them as
blockers for this feature.

| Behavior | Why |
|---|---|
| A collab token minted **before** removal keeps working on the collab server for up to 24h | JWTs are validated independently and there is no revocation path yet |
| **Re-adding** Robin to Acme with Casey as coach fails with "Coaching relationship already exists" | The surviving relationship trips the `coaching_relationships_coach_coachee_org` unique index; the re-add path has not been taught to reuse it |

## 11. Sign-off checklist

- [ ] A: removal with real coaching history returns 204
- [ ] B: exactly one `user_roles` row deleted, all other rows intact
- [ ] C: removed user gets 403 on every row of the API table
- [ ] D: coach still gets 200 on the same rows, content unchanged
- [ ] E: other-organization membership and access unaffected
- [ ] F: removing a coach behaves identically
- [ ] G: self-removal, last-admin, non-member and non-admin guards intact
- [ ] H: removed user's open SSE stream receives no further events
