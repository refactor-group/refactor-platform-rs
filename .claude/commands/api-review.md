Review these Axum API handlers and routes for:

**RESTFUL ENDPOINT DESIGN**
- Resource-based URLs (nouns, not verbs): `/users`, `/coaching-sessions`
- Proper HTTP method semantics:
  - GET: Read (idempotent, cacheable)
  - POST: Create new resource
  - PUT: Full replacement of resource
  - PATCH: Partial update of resource
  - DELETE: Remove resource
- Nested resources for relationships: `/users/{id}/coaching-sessions`
- Consistent pluralization of resource names
- Use query params for filtering/sorting: `?status=active&sort=created_at`
- Avoid action verbs in URLs (prefer `POST /orders/{id}/cancel` over `POST /cancelOrder`)

**URL NAMING CONVENTIONS**
- Use kebab-case for multi-word resources: `/coaching-sessions` not `/coachingSessions`
- Use path parameters for identifiers: `/users/{user_id}`
- Use query parameters for optional filters: `?include=relationships`
- Version APIs appropriately: `/api/v1/...` or header-based

**RESPONSE PATTERNS**
- Return created resource on POST (with 201 + Location header)
- Return updated resource on PUT/PATCH (with 200)
- Return 204 No Content on successful DELETE
- Use 404 for missing resources, 410 for deliberately removed
- Pagination pattern: `{ data: [...], meta: { total, page, per_page } }`

**HANDLER PATTERNS**
- Proper use of extractors (State, Path, Query, Json)
- Extractor ordering (rejection-prone extractors last)
- Response type consistency
- Handler function size and complexity

**REQUEST VALIDATION**
- Input validation with serde/validator
- Request body size limits
- Query parameter validation
- Path parameter type safety

**DATABASE OPERATIONS**
- SeaORM query patterns
- N+1 query prevention
- Transaction usage for multi-step operations
- Connection pool usage (avoid holding connections)

**ERROR HANDLING**
- Proper error type conversions
- AppError/ApiError pattern usage
- Error response consistency
- Logging of errors without exposing internals

**AUTHENTICATION & AUTHORIZATION**
- Middleware placement and ordering
- Token validation
- Permission checks at handler level
- Session management

**CORS & SECURITY HEADERS**
- CORS configuration appropriateness
- Security headers (CSP, X-Frame-Options, etc.)
- Cookie settings (HttpOnly, Secure, SameSite)

**RESPONSE FORMATTING**
- Consistent JSON structures
- Proper HTTP status codes
- Content-Type headers

Focus on security, performance, and maintainability.
