# Test Plan: Manually Testing In-Place Organization Member Role Changes

Verify that an organization admin can read a member's role and change it in place,
that the change takes effect immediately, and that nobody who should not be able to
reach these endpoints can.

Two new endpoints on the existing role path:

```
GET /organizations/{organization_id}/users/{user_id}/role
PUT /organizations/{organization_id}/users/{user_id}/role
```

The single most important property: **the write endpoint grants and revokes
privilege.** Section E is the section that matters most, and E4 (a plain member
promoting themselves) is the single most important case in this document. A blanket
authorization bug would show up there and nowhere else, which is why every refusal
below has a matching success elsewhere.

> [!WARNING]
> These scenarios change real membership rows. Use a local stack with disposable
> data. Several sections mutate the fixture deliberately and tell you how to restore
> it.

> [!IMPORTANT]
> **The role is PascalCase on the wire and lowercase in Postgres.** Send
> `{"role":"Admin"}`. Sending `{"role":"admin"}`, which is what the database column
> and every SQL query show, returns 422 with
> `unknown variant 'admin', expected one of 'User', 'Admin', 'SuperAdmin'`. That 422
> is easy to misread as a validation-logic bug. It is not.

## 1. Prerequisites

- Backend on branch `feat/organization-member-role-endpoints`, migrations applied.
- An API client that keeps a cookie jar per actor, e.g. `curl -b/-c`.
- Every request needs the `x-version` header, or `CompareApiVersion` rejects it with
  400 before any handler runs.

### Helpers

```sh
BASE=http://localhost:4000
VER='x-version: 1.0.0-beta1'

# Log in and keep a per-actor cookie jar. $1 = jar name, $2 = email, $3 = password.
login() { curl -s -c "/tmp/$1.jar" -X POST "$BASE/login" \
  -H 'content-type: application/x-www-form-urlencoded' \
  --data-urlencode "email=$2" --data-urlencode "password=$3" -o /dev/null -w '%{http_code}\n'; }

# Read a role.  $1 = jar, $2 = org id, $3 = user id
getrole() { curl -s -w '\n%{http_code}\n' -b "/tmp/$1.jar" -H "$VER" \
  "$BASE/organizations/$2/users/$3/role"; }

# Change a role.  $4 = Admin | User, PascalCase.
putrole() { curl -s -w '\n%{http_code}\n' -b "/tmp/$1.jar" -H "$VER" \
  -H 'content-type: application/json' -X PUT \
  "$BASE/organizations/$2/users/$3/role" -d "{\"role\":\"$4\"}"; }
```

### Fixture

The seeded development database already has the shape this plan needs. Derive the ids
rather than hardcoding them, so the plan survives a database rebuild:

```sh
q() { PGPASSWORD=password psql -h localhost -U refactor -d refactor -tA -c "$1"; }

ORG_RG=$(q  "SELECT id FROM refactor_platform.organizations WHERE name = 'Refactor Group';")
ORG_ACME=$(q "SELECT id FROM refactor_platform.organizations WHERE name = 'Acme Corp';")

U_JIM=$(q   "SELECT id FROM refactor_platform.users WHERE email = 'jim@refactorgroup.com';")
U_ROOT=$(q  "SELECT id FROM refactor_platform.users WHERE email = 'admin@refactorcoach.com';")
U_CALEB=$(q "SELECT id FROM refactor_platform.users WHERE email = 'calebbourg2@gmail.com';")
U_JAMES=$(q "SELECT id FROM refactor_platform.users WHERE email = 'james.hodapp@gmail.com';")
U_JCD=$(q   "SELECT id FROM refactor_platform.users WHERE email = 'jcdixon@proton.me';")

FAKE=00000000-0000-0000-0000-000000000000
```

Roles as seeded:

| Who | Refactor Group | Acme Corp | BigTable |
|---|---|---|---|
| `admin@refactorcoach.com` (**root**) | global `SuperAdmin`, no membership | | |
| `jim@refactorgroup.com` | **Admin**, the only one | | |
| `calebbourg2@gmail.com` | User | **Admin**, the only one | |
| `james.hodapp@gmail.com` | User | User | User |
| `jcdixon@proton.me` | User | | |

Three properties of this fixture carry the plan:

- **Refactor Group has exactly one admin (jim)**, so section D can exercise the
  last-admin guard without any setup.
- **caleb administers Acme but not Refactor Group**, so he is a genuine foreign-org
  admin for section E, not merely a non-admin.
- **james belongs to all three organizations**, so B1 can prove the response scopes
  its `roles` array to the path organization.

Log everyone in:

```sh
login jim   jim@refactorgroup.com   "$JIM_PASSWORD"
login root  admin@refactorcoach.com "$ROOT_PASSWORD"
login caleb calebbourg2@gmail.com   "$CALEB_PASSWORD"
login james james.hodapp@gmail.com  "$JAMES_PASSWORD"
```

## 2. Scenarios

### A. Read, intended success

| # | Actor | Request | Expect |
|---|---|---|---|
| A1 | root | `getrole root $ORG_RG $U_JAMES` | 200, `"role":"User"` |
| A2 | jim | `getrole jim $ORG_RG $U_JAMES` | 200, `"role":"User"` |
| A3 | jim | `getrole jim $ORG_RG $U_JIM` (self) | 200, `"role":"Admin"` |

A3 matters: reads have **no** self-target restriction, unlike writes. There is nothing
to lock yourself out of by reading.

Also confirm the body carries `created_at` and `updated_at`. A2's `updated_at` should
still be the seed value at this point; B2 checks that it moves.

### B. Write, intended success

| # | Actor | Request | Expect |
|---|---|---|---|
| B1 | jim | `putrole jim $ORG_RG $U_JAMES Admin` | 200 |
| B2 | jim | `getrole jim $ORG_RG $U_JAMES` | 200, `"role":"Admin"`, and `updated_at` **later than in A2** |
| B3 | jim | `putrole jim $ORG_RG $U_JAMES User` | 200, Refactor Group still has jim as admin |
| B4 | root | `putrole root $ORG_RG $U_JAMES Admin` | 200, the super admin path, no membership in the org needed |
| B5 | root | `putrole root $ORG_RG $U_JAMES User` | 200, restores the fixture |

**B1 is the org-scoping check.** james is a member of all three organizations. Inspect
the response body's `roles` array: it must contain **exactly one** entry, for Refactor
Group. If Acme or BigTable appears, `scope_roles_to_organization` was dropped and the
endpoint is disclosing which other organizations a member belongs to, which the
platform deliberately withholds elsewhere.

**B2 is the timestamp check.** `user_roles` carries no trigger, so `updated_at` only
moves if the application writes it. A stale value means the read endpoint is reporting
the day the member joined as the day their role last changed.

### C. Idempotency

| # | Actor | Request | Expect |
|---|---|---|---|
| C1 | jim | `putrole jim $ORG_RG $U_JCD User` while jcd is already User | 200, no-op |
| C2 | jim | `putrole jim $ORG_RG $U_JCD Admin`, twice | 200 both times |

After C2, exactly one row must exist. A second row would mean the handler inserted
instead of updating:

```sh
q "SELECT count(*) FROM refactor_platform.user_roles
   WHERE user_id = '$U_JCD' AND organization_id = '$ORG_RG';"   # must be 1
```

C1 must also write **nothing**. Note `updated_at` before and after; it must not move,
and no new `user_role_changes` row may appear.

Restore with `putrole jim $ORG_RG $U_JCD User`.

### D. Last-admin protection

Run in order; each depends on the previous. This proves the guard reads live state
rather than a fixture assumption.

| # | Actor | Request | Expect |
|---|---|---|---|
| D1 | root | `putrole root $ORG_RG $U_JIM User` | **409** `last_organization_admin`, jim is the only admin |
| D2 | root | `putrole root $ORG_RG $U_JCD Admin` | 200, two admins now |
| D3 | root | `putrole root $ORG_RG $U_JIM User` | 200, the same call that failed in D1 |
| D4 | root | `putrole root $ORG_RG $U_JCD User` | **409**, jcd inherited "only admin" |

Restore: `putrole root $ORG_RG $U_JIM Admin`, then
`putrole root $ORG_RG $U_JCD User`.

### E. Authorization, must be refused

Every row here is an actor who must not reach the endpoint.

| # | Actor | Request | Expect | Why it matters |
|---|---|---|---|---|
| E1 | *(no cookie)* | `GET .../$U_JAMES/role` | 401 | the `require_auth` layer |
| E2 | *(no cookie)* | `PUT .../$U_JAMES/role` | 401 | |
| E3 | james (plain member) | `getrole james $ORG_RG $U_JIM` | 403 | a member cannot use the admin read |
| E4 | james (plain member) | `putrole james $ORG_RG $U_JAMES Admin` | **403** | **self-promotion, the most important case here** |
| E5 | james | `putrole james $ORG_RG $U_JIM User` | 403 | a member cannot demote an admin |
| E6 | caleb (Acme admin) | `getrole caleb $ORG_RG $U_JAMES` | 403 | admin rights do not cross organizations |
| E7 | caleb | `putrole caleb $ORG_RG $U_JAMES Admin` | **403** | cross-org write, second most important |
| E8 | jim | `putrole jim $ORG_RG $U_JIM User` (self) | 403 | self-demotion blocked, matching `DELETE .../role` |
| E9 | jim | `putrole jim $ORG_RG $U_JIM Admin` (self, no-op) | 403 | self-target refused **regardless of direction** |
| E10 | jim (RG admin) | `putrole jim $ORG_ACME $U_JAMES Admin` | 403 | a foreign org id in the path must not work |

E9 is not redundant with E8. Blocking only demotion would leave an admin able to
re-grant themselves something, and the direction of a change is not what makes
self-targeting wrong.

### F. Existence and visibility masking

Two separate guarantees.

**F-a. An authorized admin gets a uniform 404**, and cannot learn whether a user
exists outside their organization.

| # | Actor | Request | Expect |
|---|---|---|---|
| F1 | jim | `getrole jim $ORG_RG $FAKE` | 404 |
| F2 | jim | `putrole jim $ORG_RG $FAKE Admin` | 404 |
| F3 | jim | `getrole jim $ORG_RG $U_CALEB` after removing caleb from RG | **404, not 403** |

For F3, first `curl -X DELETE` caleb's Refactor Group membership, or run
`putrole` against a user who is genuinely not in the org. **F3 and F1 must be
byte-identical**: same status, same body. Diff them explicitly. If a real user id
answers differently from a fabricated one, any org admin can enumerate platform user
ids. Both should be the literal body `NOT FOUND`.

**F-b. An unauthorized caller gets a uniform 403, never a 404.** The target is never
looked up at all.

| # | Actor | Request | Expect |
|---|---|---|---|
| F4 | james (member) | `getrole james $ORG_RG $U_JIM` (real user) | 403 |
| F5 | james | `getrole james $ORG_RG $FAKE` (fabricated user) | **403, not 404** |
| F6 | caleb (foreign admin) | `putrole caleb $ORG_RG $U_JAMES Admin` | 403 |
| F7 | james | `getrole james $FAKE $U_JIM` (fabricated **organization**) | **403, not 404** |

F5 vs F4 is the user-existence oracle. **F7 vs F4 is the organization-existence
oracle**, which this change closes: before it, a fabricated organization answered 404
and a real one 403, letting any authenticated user enumerate organization ids. Diff
each pair's status and body.

For contrast, a caller who *does* administer the target still gets the informative
404:

| # | Actor | Request | Expect |
|---|---|---|---|
| F8 | jim | `getrole jim $FAKE $U_JAMES` | **403**, jim does not administer a nonexistent org |
| F9 | root | `getrole root $FAKE $U_JAMES` | **404**, root passes the admin check, so the org lookup runs and reports it missing |

F8 and F9 together are the cleanest demonstration that the reorder landed: the same
request against the same nonexistent organization answers 403 to someone who cannot
administer it and 404 to someone who can. Before the change both returned 404.

### G. Input validation

| # | Actor | Body / header | Expect |
|---|---|---|---|
| G1 | jim | `{"role":"SuperAdmin"}` | 422, barred inside an organization |
| G2 | jim | `{"role":"admin"}` (lowercase) | 422, unknown variant |
| G3 | jim | `{"role":"Admin","coach_id":"<uuid>"}` | 422, `deny_unknown_fields` |
| G4 | jim | `{"role":"owner"}` | 422, unknown variant |
| G5 | jim | `{}` | 422, missing `role` |
| G6 | jim | valid body, **no** `x-version` header | 400 |
| G7 | jim | valid body, `x-version: 9.9.9` | 400 |
| G8 | jim | `GET .../not-a-uuid/role` | 400, invalid path parameter |

G3 matters beyond tidiness. Without `deny_unknown_fields` a client sending `coach_id`
gets a 200 and can reasonably believe a coach was assigned. That is exactly the
failure `POST /organizations/{id}/users` still has with its silently dropped `role`
key, tracked separately.

G1 should also emit a `role_change_denied` line in the server log naming the actor and
target. The generic 422 log carries neither.

### H. Archived organization

No organization is archived in the seed, so create the condition:

```sh
q "UPDATE refactor_platform.organizations SET archived_at = NOW() WHERE id = '$ORG_ACME';"
```

| # | Actor | Request | Expect |
|---|---|---|---|
| H1 | caleb | `getrole caleb $ORG_ACME $U_JAMES` | **200**, archiving is a write freeze, reads still work |
| H2 | root | `putrole root $ORG_ACME $U_JAMES Admin` | 409 `organization_archived` |
| H3 | caleb (Acme's own admin) | `putrole caleb $ORG_ACME $U_JAMES Admin` | 409, **not 403** |

H3 checks ordering. caleb genuinely administers Acme, so the extractor must pass and
the write freeze must be what stops him. A 403 here means the checks run in the wrong
order.

Restore: `q "UPDATE refactor_platform.organizations SET archived_at = NULL WHERE id = '$ORG_ACME';"`

### I. The change actually takes effect

A role write that does not alter what the person can do is cosmetic. Run these
**without logging anyone out**, reusing the same cookie jars throughout.

| # | Step | Expect |
|---|---|---|
| I1 | `getrole james $ORG_RG $U_JCD` | 403, baseline, same as E3 |
| I2 | `putrole jim $ORG_RG $U_JAMES Admin` | 200 |
| I3 | `getrole james $ORG_RG $U_JCD`, **same cookie** | **200** |
| I4 | `putrole james $ORG_RG $U_JCD Admin` | 200, the granted role is fully functional |
| I5 | `putrole jim $ORG_RG $U_JCD User`, then `putrole jim $ORG_RG $U_JAMES User` | 200 |
| I6 | `getrole james $ORG_RG $U_JCD`, **same cookie** | **403** |

I3 and I6 are the real assertions. If I6 still returns 200, roles are being cached in
the session rather than re-hydrated per request, and a demotion does not take effect
until the user logs out. That would be a security bug in `AuthenticatedUser`, not in
these handlers.

### J. The audit trail

Every successful change must leave exactly one row. After running section B:

```sh
q "SELECT actor_user_id, target_user_id, previous_role, new_role, changed_at
   FROM refactor_platform.user_role_changes
   WHERE target_user_id = '$U_JAMES' ORDER BY changed_at DESC LIMIT 5;"
```

| Check | Expect |
|---|---|
| B1 wrote one row | `previous_role = user`, `new_role = admin`, `actor_user_id` = jim |
| B3 wrote one row | `previous_role = admin`, `new_role = user` |
| C1 (the no-op) wrote **no** row | count unchanged across C1 |
| D1 (the refused 409) wrote **no** row | a refusal is not a change |

Neither `previous_role` nor `new_role` may be null on a change. Null on one side means
a first grant or a removal, which is what `POST` and `DELETE` record.

### K. Concurrency, needs a real Postgres

Not reachable through the mock suite. `count_admins_in_organization` already takes
`FOR UPDATE`; this confirms the new demote path calls it **inside** the transaction,
so the lock survives to the commit.

1. Two `psql` sessions. `BEGIN` in both, then in each run:
   ```sql
   SELECT * FROM refactor_platform.user_roles
   WHERE organization_id = '<rg>' AND role = 'admin' FOR UPDATE;
   ```
   The second must **block** until the first commits or rolls back.

2. With Refactor Group at two admins (jim and jcd), fire two simultaneous demotes
   from the root session, one per admin:
   ```sh
   putrole root $ORG_RG $U_JIM User & putrole root $ORG_RG $U_JCD User & wait
   ```
   Exactly one must succeed and the other must return 409. Then:
   ```sh
   q "SELECT count(*) FROM refactor_platform.user_roles
      WHERE organization_id = '$ORG_RG' AND role = 'admin';"   # must be >= 1
   ```
   A result of 0 means the count is running outside a transaction and the lock is
   released before the guard acts. Repeat a dozen times if the first attempt does not
   interleave.

3. Repeat step 2 with one `PUT` demote and one `DELETE .../role` running
   concurrently. Both go through `count_admins_in_organization`, so they contend on
   the same rows and one must lose.

## 3. Restoring the fixture

```sh
putrole root $ORG_RG $U_JIM   Admin
putrole root $ORG_RG $U_JAMES User
putrole root $ORG_RG $U_JCD   User
q "UPDATE refactor_platform.organizations SET archived_at = NULL;"
```

Then confirm against the table in section 1. Note that `user_role_changes` is
append-only by database privilege and is **not** restored; the rows this plan writes
are a permanent record of the test run, which is the intended behavior.

## 4. Known limitations, not defects

- `POST /organizations/{organization_id}/users` still accepts a `role` key in its body
  and silently discards it, always creating a plain member. Pre-existing and tracked
  separately; serde refuses `deny_unknown_fields` alongside `flatten`.
- The served OpenAPI document still carries many dangling schema references (`Id`,
  `Version`, `DateTimeWithTimeZone` and others). Also pre-existing and tracked
  separately. The schemas these two endpoints publish do resolve, which a regression
  test pins.
