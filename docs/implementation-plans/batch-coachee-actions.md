# Batch Coachee Actions Endpoints

## Problem

The frontend currently calls `GET /users/{coachee_id}/actions?scope=assigned` for every coachee in parallel when a coach views the "Coachee Actions" dashboard. With 10+ coachees, this overwhelms the backend DB connection pool (each request uses up to 3 connections for auth + protect + handler), causing `ConnectionAcquire(Timeout)` errors, 500s, and cascading 401s that trigger logout.

## Solution

Two new endpoints under the coaching relationship resource, following the existing `goal_progress` pattern:

### Endpoint 1: Single Relationship Actions
```
GET /organizations/{org_id}/coaching_relationships/{rel_id}/actions
```
Actions for a specific coaching relationship. Replaces per-coachee calls with a properly domain-scoped route.

### Endpoint 2: Batch Coachee Actions
```
GET /organizations/{org_id}/coaching_relationships/coachee-actions
```
Actions for ALL coaching relationships where the authenticated user is the coach within the organization, grouped by coachee user ID.

### Query Parameters (shared by both endpoints)

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `status` | `Option<Status>` | none | Filter by action status (PascalCase, e.g. `InProgress`, `NotStarted`) |
| `assignee_filter` | `Option<String>` | `"all"` | `"all"`, `"assigned"`, `"unassigned"` |
| `sort_by` | `Option<String>` | `"due_by"` | `"due_by"`, `"created_at"`, `"updated_at"` |
| `sort_order` | `Option<String>` | `"asc"` | `"asc"` or `"desc"` |

### Response Shapes

**Endpoint 1** — flat list:
```json
{
  "status_code": 200,
  "data": [ActionWithAssignees, ...]
}
```

**Endpoint 2** — grouped by coachee:
```json
{
  "status_code": 200,
  "data": {
    "coachee_actions": {
      "<coachee_user_id>": [ActionWithAssignees, ...],
      "<coachee_user_id>": []
    }
  }
}
```

Every coachee gets a key even if they have no matching actions (empty array).

### Authorization
- **Endpoint 1**: Authenticated user must be coach OR coachee in the relationship (reuse existing org membership + relationship participant check)
- **Endpoint 2**: Authenticated user must be an org member; only returns relationships where they are the coach

## Implementation Steps

### Step 1: Create feature branch
- Branch off `main` with name `batch-coachee-actions`

### Step 2: Entity API layer — `entity_api/src/action.rs`
- Add `find_by_coaching_relationship()` — queries actions joined through coaching_sessions for a single relationship, with status/assignee_filter/sort params
- Add `find_by_coach_relationships()` — takes a `Vec<Id>` of relationship IDs, returns `HashMap<Id, Vec<ActionWithAssignees>>` keyed by coachee user ID
- Both functions should batch-fetch assignees in a single query via `actions_user::find_assignees_for_actions()`
- Reuse existing `AssigneeFilter` post-query filtering logic from `find_by_user()`

### Step 3: Domain layer — `domain/src/action.rs`
- Add thin wrappers `find_by_coaching_relationship()` and `find_by_coach_relationships()` that delegate to entity_api and convert errors to `domain::Error`
- Add helper to resolve coaching relationships for a coach within an org (or call into `domain::coaching_relationship`)

### Step 4: Web params — `web/src/params/organization/`
- Create `coachee_action.rs` with `IndexParams` struct for shared query params (status, assignee_filter, sort_by, sort_order)
- Use serde defaults consistent with existing action params (default sort: `due_by` asc)

### Step 5: Web controller — `web/src/controller/organization/coaching_relationship_controller.rs`
- Add `actions()` handler for Endpoint 1: `GET /organizations/{org_id}/coaching_relationships/{rel_id}/actions`
- Add `batch_coachee_actions()` handler for Endpoint 2: `GET /organizations/{org_id}/coaching_relationships/coachee-actions`
- Both handlers stay thin — delegate to domain layer, return `ApiResponse`

### Step 6: Protect middleware — `web/src/protect/organizations/coaching_relationships.rs`
- Add `actions()` middleware for Endpoint 1 — verify authenticated user is a participant in the relationship
- Endpoint 2 can use existing org membership check (the handler itself filters to coach-only relationships)

### Step 7: Router — `web/src/router.rs`
- Register both routes in `organization_coaching_relationship_routes()`
- Endpoint 1 at `/organizations/:organization_id/coaching_relationships/:relationship_id/actions`
- Endpoint 2 at `/organizations/:organization_id/coaching_relationships/coachee-actions`
- Apply appropriate protect middleware

### Step 8: Tests
- Unit tests for entity_api query functions
- Integration tests for both endpoints (auth, filtering, grouping, empty results)

### Step 9: Verify
- `cargo fmt`
- `cargo clippy`
- `cargo check`
- `cargo test`

## Coordination

- **Board contract**: `BatchCoacheeActions` v1 posted to coordinator blackboard
- **Frontend issue**: Connection pool exhaustion from N+1 coachee action fetches
- **Non-breaking**: Existing `/users/{id}/actions` endpoint remains unchanged
- Frontend can migrate incrementally
