# Coaching session counts by month — manual verification

The `GET /users/{user_id}/coaching_sessions/counts` endpoint aggregates monthly
counts in a caller-supplied IANA timezone via
`date_trunc('month', date AT TIME ZONE $tz)`. Behavior across DST boundaries
and across the UTC/local-month edge cannot be verified through the
`MockDatabase` unit tests because the SQL is opaque to them; the assertions
below require a real Postgres instance.

Run this against the local dev DB whenever the SQL for
`find_counts_by_month_for_user` changes, or when validating a deploy of the
endpoint.

## Setup

Connect to the local dev DB and ensure the schema is current:

```bash
psql -U refactor -h localhost -d refactor
```

```sql
SET search_path TO refactor_platform;
```

## Cross-month edge case (the motivation for the v3 `tz` parameter)

Insert a session that lands on different calendar months depending on the
viewer's timezone. `2026-06-01T02:00:00Z` is `2026-05-31T19:00` in PDT and
`2026-06-01T04:00` in CEST.

```sql
-- Pick any existing coaching_relationship_id from your local DB.
INSERT INTO coaching_sessions (id, coaching_relationship_id, date, created_at, updated_at)
VALUES (
    gen_random_uuid(),
    (SELECT id FROM coaching_relationships LIMIT 1),
    '2026-06-01T02:00:00'::timestamp,
    NOW(),
    NOW()
);
```

Run the aggregation under three zones and verify the row lands in the
expected bucket each time:

```sql
WITH bounds AS (
  SELECT '2026-05-01'::date AS from_d, '2026-06-30'::date AS to_d
)
SELECT
  'America/Los_Angeles' AS tz,
  to_char(date_trunc('month', cs.date AT TIME ZONE 'America/Los_Angeles'), 'YYYY-MM') AS month
FROM coaching_sessions cs, bounds
WHERE cs.date = '2026-06-01T02:00:00'::timestamp
UNION ALL
SELECT
  'Europe/Berlin',
  to_char(date_trunc('month', cs.date AT TIME ZONE 'Europe/Berlin'), 'YYYY-MM')
FROM coaching_sessions cs
WHERE cs.date = '2026-06-01T02:00:00'::timestamp
UNION ALL
SELECT
  'UTC',
  to_char(date_trunc('month', cs.date AT TIME ZONE 'UTC'), 'YYYY-MM')
FROM coaching_sessions cs
WHERE cs.date = '2026-06-01T02:00:00'::timestamp;
```

Expected:

| tz                  | month   |
|---------------------|---------|
| America/Los_Angeles | 2026-05 |
| Europe/Berlin       | 2026-06 |
| UTC                 | 2026-06 |

Clean up the inserted row when done:

```sql
DELETE FROM coaching_sessions WHERE date = '2026-06-01T02:00:00'::timestamp;
```

## Index usage verification

For larger datasets, confirm the planner still uses
`coaching_sessions_relationship_date` (composite) or
`coaching_sessions_date` (single-column) under the v3 query. With
`enable_seqscan = off` the plan should show an `Index Only Scan` with the
date predicate appearing as an `Index Cond`:

```sql
SET enable_seqscan = off;
EXPLAIN (ANALYZE, BUFFERS)
SELECT
    to_char(date_trunc('month', cs.date AT TIME ZONE 'America/Los_Angeles'), 'YYYY-MM') AS month,
    COUNT(*) AS count
FROM coaching_sessions cs
JOIN coaching_relationships cr ON cs.coaching_relationship_id = cr.id
WHERE cs.date >= '2025-01-01'
  AND cs.date < '2027-01-01'
  AND (cr.coach_id = '<user_uuid>' OR cr.coachee_id = '<user_uuid>')
GROUP BY date_trunc('month', cs.date AT TIME ZONE 'America/Los_Angeles')
ORDER BY date_trunc('month', cs.date AT TIME ZONE 'America/Los_Angeles') ASC;
SET enable_seqscan = on;
```

If the plan ever switches to a sequential scan on a large table where seq
scan is disabled, the indexes have drifted and need to be revisited.

## Endpoint-level smoke (after deploy)

With the server running, exercise the full request path including
authentication, the route handler, and the 400 path for an invalid `tz`:

```bash
# Valid request (substitute a real user_id and session cookie).
curl -s -b cookie.txt \
  "http://localhost:4000/users/<user_id>/coaching_sessions/counts?from_date=2026-01-01&to_date=2026-12-31&group_by=month&tz=America/Los_Angeles" \
  | jq

# Invalid timezone -> 400 invalid_timezone.
curl -s -i -b cookie.txt \
  "http://localhost:4000/users/<user_id>/coaching_sessions/counts?from_date=2026-01-01&to_date=2026-12-31&group_by=month&tz=Not/A/Timezone"
```

The invalid-tz request should return:

```json
{
  "status_code": 400,
  "error": "invalid_timezone",
  "message": "'Not/A/Timezone' is not a recognized IANA timezone identifier."
}
```
