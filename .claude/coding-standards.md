# Coding Standards

This document outlines coding conventions and standards for the refactor-platform-rs project.

## Branching

Always start new implementation work on a feature branch — never commit directly to `main`. If the work addresses a specific GitHub issue, include the issue number in the branch name:

```
<issue-num>-short-description
```

Examples:
- `226-session-scheduled-email`
- `fix/clippy-warnings`

## Rust Conventions

### Import Organization

Organize `use` statements in the following order, separated by blank lines:

```rust
// 1. Standard library
use std::collections::HashMap;
use std::sync::Arc;

// 2. External crates
use axum::{extract::State, Json};
use sea_orm::{EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};

// 3. Internal modules (crate)
use crate::domain::models::User;
use crate::service::user_service;

// ❌ Incorrect - mixed ordering
use crate::domain::models::User;
use std::sync::Arc;
use axum::Json;
```

**No imports inside functions:** All `use` statements must be placed at the top of the file, never inside function bodies. Imports inside functions reduce discoverability and make it harder to see a module's full dependency surface at a glance.

**Rationale**:
- Improves code readability by grouping related imports
- Makes it easy to identify external dependencies
- Consistent with Rust community conventions

### Naming Conventions

- Use `snake_case` for functions, variables, and modules: `get_user_by_id`, `user_service`
- Use `PascalCase` for types, traits, and enums: `UserService`, `EntityTrait`, `ConnectionState`
- Use `SCREAMING_SNAKE_CASE` for constants: `MAX_CONNECTIONS`, `DEFAULT_TIMEOUT`

```rust
// ✅ Correct
const MAX_RETRIES: u32 = 3;

enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
}

struct UserService {
    db_pool: DatabaseConnection,
}

fn get_active_users() -> Vec<User> { ... }

// ❌ Incorrect
const max_retries: u32 = 3;
enum connectionState { ... }
struct user_service { ... }
fn GetActiveUsers() -> Vec<User> { ... }
```

### Error Handling

- Use `Result<T, E>` for fallible operations
- Prefer the `?` operator for error propagation
- Avoid `.unwrap()` and `.expect()` in production code paths

```rust
// ✅ Good - proper error handling
pub async fn find_user(id: Id) -> Result<Option<User>, DbErr> {
    let user = users::Entity::find_by_id(id)
        .one(&db)
        .await?;
    Ok(user)
}

// ❌ Bad - panics on error
pub async fn find_user(id: Id) -> User {
    users::Entity::find_by_id(id)
        .one(&db)
        .await
        .unwrap()
        .unwrap()
}
```

### Error Variant Reuse

**CRITICAL:** Before adding a new error variant, check whether an existing generic variant can carry the information you need. Adding resource-specific or feature-specific error variants (e.g., `ActiveGoalLimitReached`, `MaxSessionsExceeded`, `DuplicateSlugDetected`) causes enum proliferation and forces changes at every layer in the error chain.

**Prefer generic, reusable error variants that carry context via fields:**

```rust
// ✅ Good - generic variant, specific context in fields
EntityApiErrorKind::ValidationError {
    message: "A coaching relationship can have at most 3 in-progress goals.".into(),
    details: Some(serde_json::json!({ "in_progress_goals": summaries })),
}

// ✅ Good - different validation, same variant
EntityApiErrorKind::ValidationError {
    message: "Coach and coachee must belong to the same organization.".into(),
    details: None,
}

// ❌ Bad - one-off variant per validation rule
EntityApiErrorKind::ActiveGoalLimitReached { active_goals: Vec<GoalSummary> }
EntityApiErrorKind::DuplicateCoachingRelationship
EntityApiErrorKind::CoachOrgMismatch
```

**When IS a new variant warranted?**

A new variant is appropriate when the error represents a **fundamentally different category** that requires different handling at higher layers (different HTTP status code, different retry behavior, different logging level). Examples of good distinct variants: `RecordNotFound` (→ 404), `SystemError` (→ 503), `RecordUnauthenticated` (→ 401).

**Rule of thumb:** If two errors would map to the same HTTP status code and the same programmatic handling by the caller, they belong in the same variant with different messages/details.

### Cross-Layer Error Propagation

Errors must flow through the layer chain `entity_api` -> `domain` -> `web` without skipping layers. Each layer defines its own error types, and conversions happen at layer boundaries using `From` impls and the `?` operator.

**The error type hierarchy:**

- `entity_api::error::Error` with `EntityApiErrorKind` (e.g., `SystemError`, `RecordNotFound`)
- `domain::error::Error` with `DomainErrorKind` -> `InternalErrorKind` -> `EntityErrorKind` (e.g., `ServiceUnavailable`, `NotFound`)
- `web::Error` maps `domain::Error` to HTTP status codes via `IntoResponse`

**Rules:**

1. **Never import `entity_api` types in the `web` layer.** The web layer should only depend on `domain` types. If you find yourself importing `entity_api::error::EntityApiErrorKind` in web code, you are violating the layer boundary.

2. **Entity error types reach `domain::Error` exclusively through `From<EntityApiError>`.** Standalone `impl From<entity::*> for domain::Error` blocks are forbidden — even ones whose body internally calls `EntityApiError::from(...)`. The *signature* is the violation: it lets entity types skip the entity_api layer at call sites, and it forks the conversion logic across multiple impls instead of keeping it in the single `From<EntityApiError>` switch in `domain/src/error.rs`.

   ```rust
   // ❌ Bad — entity type reaches domain::Error directly. Forbidden even if the
   //         body routes through EntityApiError, because the signature itself
   //         exposes a cross-layer path that bypasses entity_api.
   impl From<entity::duration::OutOfRange> for domain::Error {
       fn from(err: entity::duration::OutOfRange) -> Self {
           Self::from(EntityApiError::from(err))
       }
   }

   // ✅ Good — add the variant on EntityApiErrorKind and handle it in the
   //          existing From<EntityApiError> for domain::Error switch.
   pub enum EntityApiErrorKind {
       // ...
       OutOfRange(entity::duration::OutOfRange),
   }

   impl From<entity::duration::OutOfRange> for entity_api::Error {
       fn from(err: entity::duration::OutOfRange) -> Self {
           Self { source: None, error_kind: EntityApiErrorKind::OutOfRange(err) }
       }
   }

   impl From<EntityApiError> for domain::Error {
       fn from(err: EntityApiError) -> Self {
           if let EntityApiErrorKind::OutOfRange(ref out) = err.error_kind {
               return domain::Error {
                   source: Some(Box::new(err)),
                   error_kind: DomainErrorKind::Validation(out.to_string()),
               };
           }
           // ... rest of the switch
       }
   }
   ```

   **Web layer corollary:** because web has no Cargo dep on `entity_api`, web call sites cannot write `.map_err(EntityApiError::from)?` to bridge a bare entity error. When wire-level validation needs to surface an entity-level error (e.g. validating an `Option<i16>` into a `Duration` at a controller), add a thin helper in `domain` that returns `Result<_, domain::Error>` and call it from web. The helper owns the `entity_api::Error::from(...).into()` conversion internally. See `domain::coaching_session::parse_duration_minutes` for the canonical example.

3. **Adding a new error variant** requires changes at each layer and should be rare — see [Error Variant Reuse](#error-variant-reuse) above. First check whether an existing variant (e.g., `ValidationError`) can carry your context. If a genuinely new category is needed:
   - Add the variant to `EntityApiErrorKind` in `entity_api/src/error.rs`
   - Map it to an `EntityErrorKind` variant in the `From<EntityApiError>` impl in `domain/src/error.rs`
   - Handle the `EntityErrorKind` variant in `web/src/error.rs` to return the appropriate HTTP status code

4. **Domain re-exports vs. custom wrappers.** When a domain function **only** delegates to entity_api with no added business logic — use a `pub use` re-export instead of a thin wrapper. The blanket `From` impl on `web::Error` (`impl<E> From<E> for Error where E: Into<DomainError>`) ensures entity_api errors convert correctly through the full chain.

Reserve custom domain functions for when they add value: business rules, event publishing, multi-step orchestration, or authorization logic.

```rust
// ✅ Good - pure passthrough, use a re-export
pub use entity_api::action::{create, find_by_id, delete_by_id};

// ✅ Good - adds business logic (event publishing + transaction), needs a custom function
pub async fn create(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    goal_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;
    let goal = GoalApi::create(&txn, goal_model, user_id).await?;
    link_to_created_in_session(&txn, &goal).await?;
    txn.commit().await.map_err(entity_api::error::Error::from)?;
    event_publisher.publish(GoalCreated(goal.id));
    Ok(goal)
}

// ❌ Bad - thin wrapper that only converts errors, should be a re-export
pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Ok(entity_api::some_module::find_by_id(db, id).await?)
}
```

5. **Use `domain_error_into_response()` in protect middleware** (defined in `web/src/error.rs`) to convert domain errors into HTTP responses. This routes through `web::Error`'s `IntoResponse` impl so that all error-to-status-code mapping stays in one place.

### Function Argument Limits

Clippy enforces a maximum of 7 arguments per function (`clippy::too_many_arguments`). When a function needs more, bundle related parameters into a context struct instead of adding an `#[allow]` attribute.

```rust
// ✅ Good - bundle related params into a struct
struct ActionEmailContext<'a> {
    action_body: &'a str,
    due_by: Option<DateTime<FixedOffset>>,
    session_id: Id,
    organization: &'a organizations::Model,
    goal: &'a str,
}

async fn send_email(
    config: &Config,
    assignees: &[users::Model],
    assigner: &users::Model,
    ctx: &ActionEmailContext<'_>,
) -> Result<(), Error> { ... }

// ❌ Bad - too many loose parameters
pub async fn send_email(
    config: &Config,
    assignees: &[users::Model],
    assigner: &users::Model,
    action_body: &str,
    due_by: Option<DateTime<FixedOffset>>,
    session_id: Id,
    organization: &organizations::Model,
    goal: &str,
) -> Result<(), Error> { ... }
```

### Async Patterns

- Use `async fn` for asynchronous operations
- Prefer `.await` at call sites rather than spawning tasks unnecessarily
- Be mindful of blocking operations in async contexts

```rust
// ✅ Good - async handler
pub async fn get_user(
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
) -> Result<Json<User>, AppError> {
    let user = user_service::find_by_id(&app_state.db, id).await?;
    Ok(Json(user))
}
```

### HTTP Clients (reqwest TLS)

**CRITICAL:** Every `reqwest::Client` built in production code must opt into rustls explicitly with `.use_rustls_tls()`. Never use `reqwest::Client::new()`, bare `reqwest::get(...)`, or `Client::builder().build()` without the TLS opt-in.

```rust
// ✅ Good — explicit rustls
let client = reqwest::Client::builder()
    .use_rustls_tls()
    .timeout(Duration::from_secs(30))
    .build()?;

// ❌ Bad — picks up the default backend (native-tls/OpenSSL)
let client = reqwest::Client::new();
let response = reqwest::get(url).await?;
let client = reqwest::Client::builder().timeout(...).build()?;
```

**Why:** Cargo features are unioned across the workspace. Several deps pull in reqwest with default features, which enables `default-tls` (native-tls/OpenSSL). When both backends are compiled in, `Client::new()` / `reqwest::get(...)` default to native-tls, which reads CA roots from the runtime container's system trust store. The production runtime image (`debian:bullseye-slim`) intentionally ships **without** `ca-certificates` — keeping it out minimizes attack surface and acts as a tripwire that fails loudly when this rule is violated. Rustls bundles its trust roots via `webpki-roots` and works regardless of the system store, so `.use_rustls_tls()` is the only safe choice. **Do not "fix" a TLS failure by installing `ca-certificates` in the runtime image** — fix the offending reqwest call to use rustls.

**Pre-signed download URLs:** Do not reuse a client whose `default_headers` includes `Authorization` for downloads from pre-signed URLs (e.g. S3, Recall.ai transcript downloads). Build a second header-less rustls client and store it alongside the authenticated one. See `domain::gateway::recall_ai::Provider::{client, download_client}` for the pattern.

**Review checklist:** grep for `reqwest::Client::new()`, `reqwest::get(`, and `Client::builder()` whenever touching gateway/HTTP code. The only acceptable bare-default uses are test helpers under `#[cfg(test)]` and binaries in `testing-tools/`.

### Database Transactions

Use transactions when multiple database operations must succeed or fail together (e.g., delete + insert patterns, multi-table updates). This prevents partial updates that leave data inconsistent.

```rust
use sea_orm::TransactionTrait;

let txn = db.begin().await?;
delete_all(&txn, id).await?;
insert_new(&txn, items).await?;
txn.commit().await?;  // Rolls back automatically if we never reach here
```

## Module Organization

### Layer Responsibilities

- **entity/**: SeaORM entity definitions (generated/maintained)
- **entity_api/**: CRUD operations and entity-level queries
- **domain/**: Business logic, domain models, validation rules
- **service/**: Orchestration layer, complex operations spanning multiple entities
- **web/**: HTTP handlers, request/response types, routing

### Type Re-Export Boundary

Entity types — whether table Models (e.g., `coaching_sessions::Model`) or non-table value types that constrain column values (e.g., `Provider`, `Id`, `Duration`) — are declared once in the `entity` crate and re-exported up through each layer at the `lib.rs` level. The web layer accesses them via `domain::<module>::<Type>` and **must never reach across into `entity_api` or `entity` directly**.

**The re-export chain:**

```rust
// entity/src/lib.rs — declare the module
pub mod provider;
pub mod duration;
// ...
pub type Id = Uuid;

// entity_api/src/lib.rs — re-export entity modules in the existing block
pub use entity::{
    actions, agreements, coaching_sessions, /* ... */
    provider, duration, users::Role, Id,
};

// domain/src/lib.rs — re-export entity_api re-exports in the existing block
pub use entity_api::{
    actions, agreements, coaching_sessions, /* ... */
    provider, duration, users::Role, Id,
};
```

**Access paths by layer:**

| Layer | Import for a type like `Duration` |
|---|---|
| Inside `entity_api/*.rs` | `use crate::duration::Duration;` |
| Inside `domain/*.rs` | `use crate::duration::Duration;` |
| Inside `web/*.rs` | `use domain::duration::Duration;` |

**Why this pattern:** As documented in `domain/src/lib.rs`, "consumers of the `domain` crate do not need to directly depend on the `entity_api` crate." The same isolation applies one layer down: entity_api hides direct dependence on entity. This means a layer can be refactored internally without breaking layers above it, and the type-import surface area at the web layer stays uniform regardless of where types originate.

**Common mistake:** Adding a `pub use entity::<module>::<Type>;` re-export inside a domain module file (e.g., `domain/src/coaching_session.rs`). This reaches across the entity_api boundary and bypasses the documented isolation. Re-exports of entity types belong at `lib.rs` only — never inside individual modules.

**When adding a new entity type:**

1. Declare the module in `entity/src/lib.rs`: `pub mod <name>;`.
2. Add it to the existing `pub use entity::{...};` block in `entity_api/src/lib.rs`.
3. Add it to the existing `pub use entity_api::{...};` block in `domain/src/lib.rs`.
4. Internal code in each crate imports via `crate::<name>::<Type>`. Web code imports via `domain::<name>::<Type>`.

The error-type chain (`entity_api::error` → `domain::error` → `web::Error`) is a parallel pattern with its own conversion rules — see "Cross-Layer Error Propagation" above. Both patterns share the same architectural principle: each layer hides the layers below it from the layers above.

### Thin Controllers

Controllers (web handlers) should be **thin orchestrators**: accept a request, call domain logic, and map the result to an HTTP response. Keep side-effect concerns — logging, best-effort error handling, retries — in the domain layer, not in controllers.

```rust
// ✅ Good — controller delegates side-effect handling to domain
pub async fn create(...) -> Result<impl IntoResponse, Error> {
    let user = UserApi::create_by_organization(db, org_id, model).await?;
    EmailsApi::notify_welcome_email(&config, &user).await;
    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), user)))
}

// ❌ Bad — controller handles email error logging
pub async fn create(...) -> Result<impl IntoResponse, Error> {
    let user = UserApi::create_by_organization(db, org_id, model).await?;
    if let Err(e) = EmailsApi::notify_welcome_email(&config, &user).await {
        warn!("Failed to send welcome email: {e:?}");
    }
    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), user)))
}
```

**Why**: Busy controller code obscures responsibility boundary leaks. When side-effect handling accumulates in controllers, it becomes harder to spot when they are doing work that belongs in a lower layer. Domain functions that perform best-effort operations (like sending emails) should return `()` and handle errors internally.

### Visibility Rules

- Keep module internals private by default
- Export only what's needed via `pub` or `pub(crate)`
- Use `mod.rs` or inline modules to organize related code

### Join-Table Module Encapsulation

Join-table domain modules (e.g., `coaching_session_goal`) are implementation details. Declare them `pub(crate)` and re-export their public functions through the parent domain concept (e.g., `domain::goal`). The web layer should never import a join-table module directly.

```rust
// domain/src/lib.rs
pub(crate) mod coaching_session_goal; // ✅ hidden from web layer
pub mod goal;

// domain/src/goal.rs
pub use crate::coaching_session_goal::{
    link_to_coaching_session, find_goals_by_coaching_session_id,
};

// ❌ Bad — web layer imports join-table module directly
use domain::coaching_session_goal as CoachingSessionGoalApi;
```

## Documentation

- Add doc comments (`///`) for public APIs
- Explain *why* something is done, not just *what*
- Document error conditions and edge cases

```rust
/// Finds a user by their unique identifier.
///
/// # Errors
///
/// Returns `DbErr` if the database query fails.
/// Returns `None` in the `Ok` variant if no user exists with the given ID.
pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Option<User>, DbErr> {
    // ...
}
```

## Code Review Checklist

When reviewing or writing code, ensure:

- [ ] Imports are organized (std, external, internal)
- [ ] Naming follows Rust conventions
- [ ] Error handling uses `Result` and `?` operator appropriately
- [ ] No `.unwrap()` or `.expect()` in production code paths
- [ ] Errors propagate through layers (`entity_api` -> `domain` -> `web`) without skipping
- [ ] No standalone `impl From<entity::*> for domain::Error` blocks; entity errors reach domain only via `From<EntityApiError>`
- [ ] Entity-derived types are accessed via `domain::<module>::<Type>` in web code (never `entity_api::...` or `entity::...`)
- [ ] Async operations don't block the runtime
- [ ] Public APIs have doc comments
- [ ] Every `reqwest::Client` is built with `.use_rustls_tls()`; no `reqwest::Client::new()` or bare `reqwest::get(...)` in production code
- [ ] Code passes `cargo clippy` without warnings
- [ ] Code is formatted with `cargo fmt`
