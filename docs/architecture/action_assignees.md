# Action Assignees

Actions can be assigned to one or more users (coach and/or coachee) via a many-to-many relationship.

## Data Model

```mermaid
erDiagram
    actions ||--o{ actions_users : "has assignees"
    users ||--o{ actions_users : "is assigned to"
    actions_users {
        uuid id PK
        uuid action_id FK
        uuid user_id FK
    }
```

## Layer Responsibilities

| Layer | Module | Responsibility |
|-------|--------|----------------|
| entity | `actions_users` | SeaORM entity definition |
| entity_api | `actions_user` | CRUD operations, batch queries |
| domain | `action` | `ActionWithAssignees` struct, `find_by_user()` |
| web | `action_controller` | Unified `/users/{id}/actions` endpoint |

## Key Functions

**entity_api** (`entity_api/src/actions_user.rs`):
- `set_assignees(db, action_id, user_ids)` - Atomically replaces all assignees (uses transaction)
- `find_assignees_for_actions(db, action_ids)` - Batch fetches assignees to avoid N+1 queries

**domain** (`domain/src/action.rs`):
- `find_by_user(db, user_id, query)` - Unified query with scope (assigned vs sessions) and filters

## Query Flow

```mermaid
sequenceDiagram
    participant Client
    participant Controller
    participant Domain
    participant EntityAPI
    participant DB

    Client->>Controller: GET /users/{id}/actions?scope=assigned
    Controller->>Domain: find_by_user(id, query)
    Domain->>EntityAPI: find_action_ids_by_user_id()
    EntityAPI->>DB: SELECT action_id FROM actions_users
    Domain->>EntityAPI: find_assignees_for_actions(action_ids)
    EntityAPI->>DB: SELECT * FROM actions_users WHERE action_id IN (...)
    Domain-->>Controller: Vec<ActionWithAssignees>
    Controller-->>Client: JSON response
```
