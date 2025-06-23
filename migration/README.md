# Database Migrations Guide

## Overview

This project uses a two-phase approach to database migrations:

1. **Initial Schema Setup**: The initial database schema is defined in `docs/db/refactor_platform_rs.dbml` using DBML (Database Markup Language). During the first stages of development, we used the `scripts/rebuild_db.sh` script to set up the entire schema at once. This script:
   - Converts the DBML to SQL
   - Creates necessary database objects (user, database, schema)
   - Applies the generated SQL as the first migration

2. **Incremental Migrations**: As we move into production, we utilize SeaORM's built-in migration mechanisms for all subsequent schema changes. However, the `scripts/rebuild_db.sh` script remains available for:
   - First-time local environment setup
   - Rebuilding your local database from scratch

## Setting Up Migrations

### Installing sea-orm-cli

```bash
cargo install sea-orm-cli
```

### Environment Configuration

Before running migrations, ensure you have a proper environment configuration:

1. Create either `.env` or `.env.local` in your project root with the following variables:
   ```env
   DATABASE_URL=postgres://refactor:password@localhost:5432/refactor_platform
   ```

2. Or set the environment variables directly:
   ```bash
   export DATABASE_URL=postgres://refactor:password@localhost:5432/refactor_platform
   ```

### Manual Schema Creation (Production)

For production environments the `refactor_platform` schema is **not** created automatically by SeaORM migrations.  
Create it once in your production database before running any migrations:

```sql
CREATE SCHEMA IF NOT EXISTS refactor_platform;
```

After the schema exists, run the normal migration commands (all examples below continue to use `-s refactor_platform`).

### Schema Privileges (Production)

After the schema exists, ensure the `refactor` role (used in the `DATABASE_URL`) has the proper rights; otherwise the migrator will error with `permission denied for table seaql_migrations`.

```sql
-- Allow the role to create and use objects in the schema
GRANT USAGE, CREATE ON SCHEMA refactor_platform TO refactor;

-- Transfer ownership of the SeaORM migrations tracking table
ALTER TABLE refactor_platform.seaql_migrations OWNER TO refactor;

-- Grant DML privileges on all existing tables
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA refactor_platform TO refactor;

-- Ensure future tables inherit these privileges
ALTER DEFAULT PRIVILEGES IN SCHEMA refactor_platform
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO refactor;
```

## Running Migrations

### Important: Schema Specification

All migration commands must include the `-s refactor_platform` flag to specify the correct schema. For example:

```bash
sea-orm-cli migrate up -s refactor_platform
```

### Available Commands

#### Migration Generation
```bash
# Generate a new migration
sea-orm-cli migrate generate MIGRATION_NAME -s refactor_platform
```

#### Applying Migrations
```bash
# Apply all pending migrations
sea-orm-cli migrate up -s refactor_platform

# Apply specific number of pending migrations
sea-orm-cli migrate up -s refactor_platform -n 10
```

#### Rolling Back Migrations
```bash
# Rollback last applied migration
sea-orm-cli migrate down -s refactor_platform

# Rollback specific number of migrations
sea-orm-cli migrate down -s refactor_platform -n 10

# Rollback all migrations
sea-orm-cli migrate reset -s refactor_platform
```

#### Database Operations
```bash
# Drop all tables and reapply migrations
sea-orm-cli migrate fresh -s refactor_platform

# Rollback and reapply all migrations
sea-orm-cli migrate refresh -s refactor_platform
```

#### Status Check
```bash
# Check migration status
sea-orm-cli migrate status -s refactor_platform
```

For more details, refer to the [SeaORM Migration Documentation](https://www.sea-ql.org/SeaORM/docs/migration/running-migration/).
