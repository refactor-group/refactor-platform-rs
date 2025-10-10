# Claude Code Guidelines

## Database Migrations

**CRITICAL: PostgreSQL Type Ownership** - When creating any PostgreSQL type (enum, composite, etc.) using `create_type()`, you MUST immediately follow it with `ALTER TYPE refactor_platform.<type_name> OWNER TO refactor`.

**Why this is required:** SeaORM's `create_type()` generates unqualified SQL (e.g., `CREATE TYPE role` instead of `CREATE TYPE refactor_platform.role`). PostgreSQL assigns ownership to the user executing the CREATE TYPE command. If the type is created by a superuser (like `doadmin`) but later migrations run as the `refactor` user, those migrations will fail with "must be owner of type" errors when attempting to ALTER the type.

**For existing types without proper ownership:** A database superuser must manually run `ALTER TYPE refactor_platform.<type_name> OWNER TO refactor;` before migrations can modify those types.
