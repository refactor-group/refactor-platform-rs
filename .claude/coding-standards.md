# Coding Standards

This document outlines coding conventions and standards for the refactor-platform-rs project.

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
- Create domain-specific error types when appropriate
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
- [ ] Async operations don't block the runtime
- [ ] Public APIs have doc comments
- [ ] Code passes `cargo clippy` without warnings
- [ ] Code is formatted with `cargo fmt`
