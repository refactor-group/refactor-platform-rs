# Database Indexes for Sorting

## Overview
This document outlines the required database indexes to support efficient sorting operations in the coaching sessions API.

## Coaching Sessions Sorting Indexes

The coaching sessions API supports sorting by the following fields:
- `date` - Session date/time
- `created_at` - Record creation timestamp  
- `updated_at` - Record modification timestamp

### Required Indexes

These indexes are automatically created by the SeaORM migration `m20250730_000000_add_coaching_sessions_sorting_indexes.rs`:

#### 1. Composite Index: coaching_relationship_id + date
**Name**: `coaching_sessions_relationship_date`
**Purpose**: Optimizes the most common query pattern - filtering by relationship and sorting by date
**Query Pattern**: `WHERE coaching_relationship_id = ? ORDER BY date DESC`

#### 2. Single Column Indexes

- **coaching_sessions_date**: For date-only sorting (fallback scenarios)
- **coaching_sessions_created_at**: For created_at sorting  
- **coaching_sessions_updated_at**: For updated_at sorting

## Performance Considerations

### Query Performance
- **Without indexes**: Full table scan + sorting (O(n log n))
- **With indexes**: Index scan + optional sort (O(log n) to O(k log k) where k << n)

### Index Maintenance
- Indexes add ~10-15% overhead on INSERT/UPDATE operations
- Disk space: Each index approximately 10-20% of table size
- For coaching sessions table with expected volume < 100K records, overhead is minimal

### Migration Deployment
Run the migration to create all indexes:
```bash
sea-orm-cli migrate up -s refactor_platform
```

## Validation Queries

After creating indexes, validate performance with:

```sql
-- Explain query plan (should show Index Scan)
EXPLAIN (ANALYZE, BUFFERS) 
SELECT * FROM refactor_platform.coaching_sessions 
WHERE coaching_relationship_id = 'uuid-here' 
ORDER BY date DESC;

-- Check index usage
SELECT schemaname, tablename, indexname, idx_scan, idx_tup_read, idx_tup_fetch
FROM pg_stat_user_indexes 
WHERE tablename = 'coaching_sessions';
```

## Related Files
- Migration file: `migration/src/m20250730_000000_add_coaching_sessions_sorting_indexes.rs`
- Database schema: `docs/db/refactor_platform_rs.dbml`
- API implementation: `web/src/controller/coaching_session_controller.rs`
- Query layer: `entity_api/src/query.rs`