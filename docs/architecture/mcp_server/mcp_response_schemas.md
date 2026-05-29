# MCP Request/Response Schemas
Request and Response types for MCP tool handlers. Entity models already derive `Serialize` with `#[serde(skip_serializing)]` on sensitive fields. Where a tool returns an entity model directly, no wrapper is needed. Where a tool adds computed or nested fields, a thin wrapper uses `#[serde(flatten)]` to inline the entity and only defines the extra fields.

## `get_coachee`

### Input

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| coachee_id | uuid? | Coaches: yes. Coachees: no (defaults to self). | The coachee to look up. |
| include | string[]? | No | Array of related data to inline. Valid values: `"goals"`, `"actions"`, `"notes"`. |

### `CoacheeResponse`
Flattens `users::Model` (or `coachees::Model`) and adds computed stats. No entity fields are redefined.

| Extra field | Type | Source |
|-------------|------|--------|
| active_goals_count ** | u32 | count of goals where status in (`not_started`, `in_progress`, `on_hold`) |
| open_actions_count | u32 | count of actions where status in (`not_started`, `in_progress`, `on_hold`) |
| overdue_actions_count | u32 | count of actions where status in (`not_started`, `in_progress`, `on_hold`) AND `due_by < now` |
| last_session_date | timestamptz? | most recent `coaching_sessions.date` |
| next_session_date | timestamptz? | earliest future `coaching_sessions.date` |
| goals | `goals::Model[]?` | populated when `include` contains `"goals"`, filtered to active statuses |
| actions | `actions::Model[]?` | populated when `include` contains `"actions"`, filtered to active statuses |
| notes | `notes::Model[]?` | populated when `include` contains `"notes"` |

**"Active" means status is one of: `not_started`, `in_progress`, `on_hold`. This excludes `completed` and `wont_do`. Applies to `get_coachee` stat counts and `include` arrays.

When `include` is absent or empty, `goals`, `actions`, and `notes` are omitted (not returned as empty arrays).

The `include` arrays return raw entity models directly — no wrappers, no nested refs. This keeps `get_coachee` lightweight.

## `list_actions`

Filters (all optional):

| Filter | Type | Description |
|--------|------|-------------|
| coachee_id | uuid? | Required for coaches to scope to a coachee. Defaults to self for coachees. |
| coaching_session_id | uuid? | Filter to actions from a specific session |
| keyword | string? | Searches `actions.body` text |
| date_from | date? | Actions created on or after this date |
| date_to | date? | Actions created on or before this date |
| status | string? | Filter by status (`not_started`, `in_progress`, `completed`, `on_hold`, `wont_do`) |

### `ActionResponse`
Flattens `actions::Model` and adds a session reference with frontend URL.

| Extra field | Type | Source |
|-------------|------|--------|
| session | `SessionResponse` | from `coaching_sessions` via `coaching_session_id` |

## Shared response types

### `SessionResponse`
Flattens `coaching_sessions::Model` and adds a computed frontend URL. Reused by `list_sessions` and nested inside `ActionResponse`.

| Extra field | Type | Source |
|-------------|------|--------|
| session_url | string | `{FRONTEND_BASE_URL}/coaching-sessions/{id}` (computed) |

## `list_coachees`

### Input
No parameters — the coach is identified via PAT.

### Output
Returns raw `users::Model[]` for each coachee in the coach's coaching relationships.

## `list_sessions`

### Input

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| coachee_id | uuid? | Coaches: yes. Coachees: no (defaults to self). | Scope sessions to this coachee's coaching relationship. |
| date_from | date? | No | Sessions on or after this date. |
| date_to | date? | No | Sessions on or before this date. |

### Output
Returns `SessionResponse[]` — each session with a computed `session_url`.

## `get_session`

### Input

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| coachee_id | uuid? | Coaches: yes. Coachees: no (defaults to self). | Identifies the coaching relationship. |
| session_id | uuid? | No | The session to summarize. Defaults to the most recent session for the coaching relationship. |

### Output
Returns a structured data bundle for the client LLM to summarize. No server-side LLM needed. All fields are raw entity models.

| Field | Type | Source |
|-------|------|--------|
| session | `SessionResponse` | the session with `session_url` |
| notes | `notes::Model[]` | all notes for the session |
| actions | `actions::Model[]` | all actions for the session |
| agreements | `agreements::Model[]` | all agreements for the session |
| goals | `goals::Model[]` | goals linked to the session via `coaching_sessions_goals` |
