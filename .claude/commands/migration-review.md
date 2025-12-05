Review this database migration for correctness, safety, and best practices:

**SCHEMA CHANGES**
- Table/column naming conventions (snake_case)
- Proper foreign key relationships and ON DELETE behavior
- Index usage and optimization for common queries
- Nullable vs NOT NULL decisions with sensible defaults
- Data type appropriateness (e.g., UUID vs serial, timestamptz vs timestamp)

**POSTGRESQL TYPE OWNERSHIP**
- ⚠️ CRITICAL: Verify `ALTER TYPE refactor_platform.<type_name> OWNER TO refactor` after any `create_type()`
- Schema qualification for custom types
- Enum type modifications (adding values is safe, removing is not)

**MIGRATION SAFETY**
- Reversibility: Does `down()` properly undo `up()`?
- Data preservation during ALTER operations
- Lock considerations for large tables (avoid long-held locks)
- Idempotency: Can migration be safely re-run?
- Backward compatibility with running application code

**SEED DATA**
- Development vs production seed data separation
- Referential integrity in seed data order
- Proper cleanup in down migrations
- Avoid hardcoded IDs that may conflict

**SEAORM PATTERNS**
- 📚 **Use Context7 MCP to check latest SeaORM migration API usage**
- Proper use of `manager.create_table()`, `alter_table()`, `drop_table()`
- Correct column definitions with `ColumnDef::new()`
- Foreign key constraint definitions
- Index creation with appropriate columns

**ENTITY SYNCHRONIZATION**
- Entity struct matches migration schema
- ActiveModel derives are correct
- Relation definitions match foreign keys
- Column attributes (e.g., `column_name`) are accurate

**PERFORMANCE CONSIDERATIONS**
- Large data migrations should be batched
- Index creation on large tables (consider CONCURRENTLY)
- Avoid full table scans in data migrations

**ROLLBACK PLAN**
- Is rollback tested and documented?
- Data backup considerations before destructive changes
- Feature flags for gradual rollout if needed

Provide specific feedback on migration safety and correctness.
