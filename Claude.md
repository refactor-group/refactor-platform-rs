# Claude Code Guidelines

## Database Migrations

**CRITICAL: PostgreSQL Type Ownership** - When creating any PostgreSQL type (enum, composite, etc.) using `create_type()`, you MUST immediately follow it with `ALTER TYPE refactor_platform.<type_name> OWNER TO refactor`. Without explicit ownership, migrations will fail when different database users attempt to modify the type in subsequent migrations.
