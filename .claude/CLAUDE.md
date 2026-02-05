# refactor-platform-rs Project Instructions

## Mandatory File Consultations

**Code Implementation/Editing** → Read `.claude/coding-standards.md` FIRST
**Pull Request Operations** → Read `.claude/pr-instructions.md` FIRST
**Database Migrations** → Read the Database Migrations section below FIRST

## Rules
- Project standards override global defaults on conflict
- Validate all code against standards before task completion
- PR reviews require both files if coding standards referenced
- Always run `cargo clippy` and `cargo fmt` before committing
- Always skip adding Claude attribution for commits or PRs (no "Generated with Claude Code" or "Co-Authored-By: Claude" footers)

## Naming Conventions

### No Redundant Type Prefixes
**CRITICAL:** Never use redundant prefixes in type names when the module path already provides context.

**Examples:**
- ❌ `oauth/provider.rs` → `OAuthProvider` (redundant "OAuth" prefix)
- ✅ `oauth/provider.rs` → `Provider` (module path provides context)

- ❌ `oauth/providers/google.rs` → `GoogleProvider` (redundant "Google" prefix)
- ✅ `oauth/providers/google.rs` → `Provider` (module path provides context)

- ❌ `api_key/auth.rs` → `ApiKeyProvider` (redundant "ApiKey" prefix)
- ✅ `api_key/auth.rs` → `Provider` (module path provides context)

**Import patterns:**
```rust
// ✅ Good - use module path or alias for clarity at call sites
use oauth::Provider;
use oauth::providers::google::Provider as GoogleProvider;

// ❌ Bad - redundant prefixes baked into type names
use oauth::OAuthProvider;
use oauth::providers::GoogleOAuthProvider;
```

**Rationale:** Module paths already provide full context. Redundant prefixes create noise and violate DRY principles.

## Database Migrations

**CRITICAL: PostgreSQL Type Ownership** - When creating any PostgreSQL type (enum, composite, etc.) using `create_type()`, you MUST immediately follow it with `ALTER TYPE refactor_platform.<type_name> OWNER TO refactor`.

**Why this is required:** SeaORM's `create_type()` generates unqualified SQL (e.g., `CREATE TYPE role` instead of `CREATE TYPE refactor_platform.role`). PostgreSQL assigns ownership to the user executing the CREATE TYPE command. If the type is created by a superuser (like `doadmin`) but later migrations run as the `refactor` user, those migrations will fail with "must be owner of type" errors when attempting to ALTER the type.

**For existing types without proper ownership:** A database superuser must manually run `ALTER TYPE refactor_platform.<type_name> OWNER TO refactor;` before migrations can modify those types.

## Project Structure

```
refactor-platform-rs/
├── entity/        # SeaORM entity definitions
├── entity_api/    # Entity API layer
├── domain/        # Domain logic and business rules
├── service/       # Service layer
├── web/           # Axum web handlers and routes
├── migration/     # SeaORM database migrations
└── src/           # Main application entry point
```

## Toolchain

- **Build/Run**: `cargo build`, `cargo run`
- **Testing**: `cargo test`
- **Linting**: `cargo clippy`
- **Formatting**: `cargo fmt`
- **Database**: SeaORM with PostgreSQL
- **Web Framework**: Axum
