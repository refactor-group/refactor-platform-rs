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
