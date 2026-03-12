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

2. **Adding a new error variant** requires changes at each layer and should be rare — see [Error Variant Reuse](#error-variant-reuse) above. First check whether an existing variant (e.g., `ValidationError`) can carry your context. If a genuinely new category is needed:
   - Add the variant to `EntityApiErrorKind` in `entity_api/src/error.rs`
   - Map it to an `EntityErrorKind` variant in the `From<EntityApiError>` impl in `domain/src/error.rs`
   - Handle the `EntityErrorKind` variant in `web/src/error.rs` to return the appropriate HTTP status code

3. **Domain re-exports of entity_api functions** that return `entity_api::Error` must be wrapped in a thin domain function so callers receive `domain::Error`. The `?` operator handles the conversion automatically via the existing `From` impl:

```rust
// ✅ Good - domain wrapper converts errors at the boundary
pub async fn find_by_id_with_relationship(
    db: &DatabaseConnection,
    id: Id,
) -> Result<(Model, relationships::Model), Error> {
    Ok(entity_api_module::find_by_id_with_relationship(db, id).await?)
}

// ❌ Bad - raw re-export leaks entity_api::Error into higher layers
pub use entity_api::some_module::find_by_id_with_relationship;
```

4. **Use `domain_error_into_response()` in protect middleware** (defined in `web/src/error.rs`) to convert domain errors into HTTP responses. This routes through `web::Error`'s `IntoResponse` impl so that all error-to-status-code mapping stays in one place.

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
- [ ] Async operations don't block the runtime
- [ ] Public APIs have doc comments
- [ ] Code passes `cargo clippy` without warnings
- [ ] Code is formatted with `cargo fmt`
