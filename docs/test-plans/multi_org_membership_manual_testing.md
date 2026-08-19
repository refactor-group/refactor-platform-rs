# Test Plan: Manually Testing Multi-Org Membership

Verify that an existing user can be attached to a second organization, removed
from one organization without losing their account, and notified by email.
Design/plan: [`multi-org-membership.md`](../implementation-plans/multi-org-membership.md).

The single most important property: **adding is not moving.** A user attached to
a second organization must still hold their original role in the first one.

> [!WARNING]
> Scenario F sends real mail through Resend, and Scenario D deletes real rows.
> Use a local stack with disposable data, or dedicated test accounts.

## 1. Prerequisites

- Backend on branch `feat/multi-org-membership`, frontend on its matching branch
  of the same name. They must ship together: the new UI calls routes that do not
  exist on the current backend.
- At least two organizations and four or more users, including:
  - a SuperAdmin account,
  - a user who is Admin of two organizations (create in Scenario B),
  - a plain (non-admin) member.
- Email (Scenario F only) requires a Resend template plus
  `ADDED_TO_ORGANIZATION_EMAIL_TEMPLATE_ID` set in the backend environment.
  **Without it no email sends and the backend logs a config warning. That is
  expected, not a bug.** Every other scenario works without it.

Template variables the backend supplies:

| Variable | Value |
|---|---|
| `first_name` | added user's first name |
| `last_name` | added user's last name |
| `organization_name` | organization they were added to |
| `role_name` | `Admin` or `Member` |
| `inviter_first_name` | first name of whoever added them |
| `inviter_full_name` | full name of whoever added them |
| `organization_url` | link into the app |

Note on role naming: the UI says **Member**, the JSON wire value is `"User"`,
and the database stores `user`. All three refer to the same role.

## 2. Scenario A: the driving use case, as SuperAdmin

Logged in as SuperAdmin.

1. Create a new organization (for example *BigTable*).
2. Open it, go to **Members → Add member → Add existing member**.
3. Enter the email of a user who already belongs to a different organization,
   press **Find**.
4. Confirm the returned card shows the right person (name and email).
5. Set role **Admin**, press add. Expect a success toast and the member list to
   refresh.

Then verify both halves:

| Check | Expected |
|---|---|
| New organization's member list | the user appears, Admin badge |
| **Original organization's member list** | the user **still appears, original role unchanged** |

The second row is the whole point of the feature. A naive implementation moves
the user instead of adding them, and only this check catches it.

6. Repeat for a second existing user, role **Member**.
7. In the new organization, assign the first user as the second user's **coach**.
   The relationship must save.

Step 7 is the real proof the membership rows are correct: coaching-relationship
creation independently validates that both parties belong to the organization,
so it fails if either role row is wrong or missing.

8. Use the OrganizationSwitcher to move between the two organizations and
   confirm both render correctly.

## 3. Scenario B: the org-admin path, as a NON-SuperAdmin

Scenario A only exercises the SuperAdmin half. Org admins reach this feature too
and must be tested deliberately.

Setup: make a test user **Admin of two organizations** (call them Org1 and Org2).
Log in as that user, not as SuperAdmin.

| Step | Expected |
|---|---|
| Add a member of Org1 into Org2 via *Add existing member* | succeeds, same as Scenario A |
| Look up the email of a user who belongs only to an organization this admin does **not** administer, **and who was never a member of one they do** | **"No user found with that email."** |
| Look up a completely nonexistent email address | the identical message |

Those last two responses being **indistinguishable is deliberate**. It is
anti-enumeration behavior: an org admin must not be able to probe whether an
arbitrary email has an account. Do not report it as a bug.

> [!IMPORTANT]
> The "never a member of one they do" clause is load-bearing. A **former** member
> of an organization this admin administers **is** found, by design. That is
> [Scenario G](#8-scenario-g-re-adding-a-former-member), and it is the one case
> where this guarantee is deliberately narrower than it reads here. Test it with
> someone who has no history in the admin's organizations at all.

Then log in as a **plain member** (not an admin) of an organization:

- the **Add existing member** tab must be absent entirely.

## 4. Scenario C: the cross-organization data leak check

This is invisible from the UI. Inspecting the raw response is the only way to
catch it.

With a user now belonging to two organizations, log in as an admin of one of
them and fetch the member list, then read the raw JSON.

Either open browser devtools → Network → the `GET /organizations/{id}/users`
request → Response, or call it directly with the session cookie:

```bash
curl -sS "$BASE/organizations/$ORG_ID/users" -H "Cookie: id=$SESSION_COOKIE" | jq
```

For the multi-org user, check the `roles` array:

| Must contain | Must NOT contain |
|---|---|
| that organization's role row | any role row for another organization |
| any global SuperAdmin role (`organization_id` is null) | any other organization's UUID |

Repeat from the other organization's member list. Each response must show only
its own organization's role.

Before this feature every user belonged to exactly one organization, so this
could not leak. Multi-org membership is what makes it possible, and a regression
here silently discloses every member's other organizations to every org admin.

## 5. Scenario D: removal semantics

Two different destructive actions exist on a member card. They must stay
distinct.

| Action | Target | Expected |
|---|---|---|
| **Remove from organization** | a member of two organizations, **with no sessions in this one** | removed from this organization only; account intact; still in the other organization; coaching relationships **in the other organization survive** |
| **Remove from organization** | a member **with at least one past or future session in this organization** | **succeeds.** Their sessions, notes and actions survive; removal revokes their access to them rather than destroying them |
| **Remove from organization** | a **coach who has coachees** in this organization | succeeds. Coach and coachee are treated identically; reassigning the coachees afterwards is the intended flow |
| **Delete** | a member of two organizations | refused with a **409** and a message telling you to remove them from the organization instead |
| **Remove from organization** | the organization's only remaining Admin | refused with a clear last-admin message |
| Either action | yourself | not offered / refused |

After the successful remove, re-check the other organization's member list and
its coaching relationships explicitly. "Remove" must not cascade.

**The session cases must be exercised against a real database.** `coaching_sessions`
references `coaching_relationships` with NO ACTION while goals and session series
cascade, so a mock-backed test cannot reach either behavior. Use an account that
has actually had a session scheduled, not a freshly created one.

> [!NOTE]
> The two session rows above previously expected a **409 `user_has_coaching_history`**.
> That error variant was deleted with rs#377 and can no longer be emitted; removal
> now succeeds for a member with history, and revokes their access instead. Verify
> revocation with an API client, not the UI: the organization disappearing from the
> switcher was already true before rs#377, while access remained wide open. See
> `remove_org_member_revokes_access_manual_testing.md`.

## 6. Scenario E: regression checks on pre-existing flows

The authorization mechanism behind three already-existing routes was rewired.
There is no intended behavior change, and the point of this scenario is proving
that.

Logged in as an **org admin**:

| Flow | Expected |
|---|---|
| Create a brand-new member (the *Create new member* tab) | works as before |
| **Resend an invite** to a pending member | works as before, invite email arrives |
| Delete a member who belongs to only one organization | works as before |

Then log in as a **plain member** of the same organization and confirm all three
are refused with permission denied.

**Resend invite is the most important line in this table.** Its automated test
can only prove the request got past authorization and reached the handler, not
that the handler succeeds, because the actual send needs live Resend
configuration. Manual confirmation is the only coverage it has.

## 7. Scenario F: email delivery

Requires `ADDED_TO_ORGANIZATION_EMAIL_TEMPLATE_ID` and the Resend template
(see [§1](#1-prerequisites)).

1. Add an existing user to an organization as **Admin**. Confirm the email
   arrives with the correct organization name and the role reading `Admin`.
2. Add another existing user as **Member**. Confirm the role reads `Member`.
3. Check the inviter's name in the body matches the account that performed the
   add.

**No automated test covers the call site.** The handler could stop sending
entirely and the whole backend suite would still pass. This manual check is the
only thing standing between a working email and a silent regression.

If no email arrives, first check the backend log for the missing-template-id
config warning before treating it as a defect:

```bash
docker compose logs backend | grep -i 'added_to_organization'
```

## 8. Scenario G: re-adding a former member

Removing a member used to be a one-way door for the admin who did it. This
scenario is the recovery path, and it must be run **as an org admin, never as a
SuperAdmin**, because a SuperAdmin could always do this and testing as one proves nothing.

Setup: log in as an admin of exactly one organization (Org1). Pick a member of
Org1 who is **not** a member of any other organization you administer.

| # | Step | Expected |
|---|---|---|
| G1 | Remove that member from Org1 | succeeds |
| G2 | Search their email in *Add existing member* | **found.** Before this change it returned "No user found with that email." |
| G3 | Add them back to Org1 | succeeds |
| G4 | Confirm their role and coaching relationships | membership restored; the surviving coaching relationship is reused, not duplicated |

**G2 is the whole scenario.** If it reports no user found, the fix is not working,
and the failure looks exactly like the bug it replaces.

### What must still be refused

Run these as the same org admin. Each proves the reach did not widen too far.

| # | Step | Expected |
|---|---|---|
| G5 | Search the email of a user who has never been in any organization you administer | **"No user found with that email."** |
| G6 | Search a completely nonexistent email | the identical message |
| G7 | Search a user who was removed from an organization you do **not** administer | **"No user found with that email."** |
| G7b | Immediately repeat G7 **as a SuperAdmin** | **found.** The control. |

G7 is the security case. Being a former member *somewhere* must not surface
someone; only former membership in **your** organization does. If G7 finds them,
the reach has become a platform-wide email oracle and that is a defect worth
stopping the release for.

> [!NOTE]
> **G7 needs setup, and a seeded database will not have it.** The audit table
> only carries history for organizations that have actually seen a removal. If no
> removal has ever happened outside the organizations you administer, G7 passes
> vacuously and proves nothing. Create the history first, as a SuperAdmin:
>
> ```sh
> # as root, remove a member from an organization the G-actor does NOT administer
> curl -s -X DELETE -b /tmp/root.jar -H "$VER" \
>   "$BASE/organizations/$OTHER_ORG/users/$SOMEONE/role"
> ```
>
> Then confirm the row landed before running G7:
>
> ```sql
> SELECT o.name, previous_role, new_role
> FROM refactor_platform.user_role_changes urc
> JOIN refactor_platform.organizations o ON o.id = urc.organization_id
> WHERE urc.target_user_id = '<someone>';
> ```
>
> **G7b is not optional.** A zero result is equally consistent with correct
> scoping, a typo'd email, a missing fixture, or a lookup broken for everyone.
> Only a SuperAdmin finding the same email in the same breath makes the zero mean
> what G7 claims it means. Restore the membership afterwards.

### Known limitation, not a defect

**Removals that predate 2026-08-16 are invisible to this.** The recovery reads the
`user_role_changes` audit table, which did not exist before then, and the
`user_roles` rows it would have described are already deleted, so the history
cannot be reconstructed. A member removed before that date still needs a
SuperAdmin to re-add them, and G2 will report no user found for them.

Check before filing a bug:

```sql
SELECT changed_at, previous_role, new_role
FROM refactor_platform.user_role_changes
WHERE target_user_id = '<user>' ORDER BY changed_at;
```

No rows means this scenario cannot apply to that user.

### Rate limiting

The lookup is throttled per requester, 30 attempts per hour.

| # | Step | Expected |
|---|---|---|
| G8 | Search 31 times in quick succession as one admin | the 31st returns **429** `user_lookup_rate_limited` |
| G9 | Search once as a **different** admin immediately after | succeeds; the limit is per requester, not global |
| G10 | Confirm a request refused with 403 records nothing | a non-admin hitting the endpoint must not consume anyone's allowance |

## 9. Sign-off checklist

- [ ] A: user added to a second organization **and still in the first**
- [ ] A: coaching relationship saves in the new organization
- [ ] B: org admin can add across their own two organizations
- [ ] B: out-of-scope email is indistinguishable from a nonexistent one
- [ ] B: plain member sees no *Add existing member* tab
- [ ] C: raw `roles` JSON leaks no other organization's UUID, both directions
- [ ] D: remove keeps the account and other memberships; delete 409s; last-admin refused
- [ ] G: an org admin can re-add a member they removed, and the search finds them
- [ ] G: a user with no history in the admin's organizations stays invisible (G5, G7)
- [ ] G: the lookup throttles at 30/hour per requester, not globally
- [ ] E: create, resend invite, and single-org delete all still work for an admin, all 403 for a member
- [ ] F: email arrives with the right organization name and role wording
