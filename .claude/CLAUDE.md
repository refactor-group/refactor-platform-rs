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
├── docs/              # Architecture docs and implementation plans
├── domain/            # Domain logic and business rules (emails, users, sessions)
├── entity/            # SeaORM entity definitions
├── entity_api/        # Entity API layer (CRUD operations)
├── events/            # SSE domain event definitions
├── meeting-ai/        # Meeting AI abstraction (recording bots, transcription, analysis)
├── meeting-auth/      # OAuth 2.0 and API key authentication for meeting providers
├── migration/         # SeaORM database migrations
├── nginx/             # Production nginx configuration
├── nginx-preview/     # PR preview nginx configuration
├── scripts/           # Database rebuild and utility scripts
├── service/           # Service layer (config, app state)
├── src/               # Main application entry point
├── sse/               # SSE server and event handling
├── testing-tools/     # Test helpers and scenario builders
└── web/               # Axum web handlers, routes, and middleware
```

## Toolchain

- **Build/Run**: `cargo build`, `cargo run`
- **Testing**: `cargo test`
- **Linting**: `cargo clippy`
- **Formatting**: `cargo fmt`
- **Database**: SeaORM with PostgreSQL
- **Web Framework**: Axum

## PR Preview Environments

### Reusable Workflow
`ci-deploy-pr-preview.yml` is the central reusable workflow called by both frontend and backend PR workflows. It builds Docker images, deploys per-PR container stacks via SSH, and posts preview URLs as PR comments.

### Docker Compose Stack
Each PR gets an isolated stack defined in `docker-compose.pr-preview.yaml`:
- **postgres**: Per-PR database with isolated schema
- **backend**: Axum API server (port allocated per PR)
- **frontend**: Next.js with `basePath=/pr-<NUM>`
- **migrator**: Runs SeaORM migrations then exits
- **nginx**: Containerized reverse proxy for path-based routing

### Nginx Routing
Nginx runs as a Docker container (`docker-compose.nginx-preview.yaml`) using Docker's internal DNS (`resolver 127.0.0.11`) to resolve container names. Path-based routing:
- `/pr-<NUM>/api` → health check JSON response
- `/pr-<NUM>/api/<path>` → proxied to backend container
- `/pr-<NUM>/` → proxied to frontend container (passes `$request_uri` for basePath)

### CORS Wildcard Handling
`web/src/lib.rs` uses `AllowOrigin::mirror_request()` when `ALLOWED_ORIGINS` contains `*`. This mirrors the request's `Origin` header instead of returning `Access-Control-Allow-Origin: *`, which browsers reject when credentials are included.

### entrypoint.sh Schema Flow
The entrypoint waits for PostgreSQL readiness, then idempotently creates the `refactor_platform` schema and sets `search_path`. This supports the PR preview migrator container which needs the schema to exist before running migrations.

### Manual Dispatch with Commit Selection
`dispatch-pr-preview.yml` allows manual PR preview deployments with specific commit combinations. Users select backend and frontend commits from dropdown menus (auto-populated by `refresh-preview-commits.yml`) and enter a PR number. The workflow validates the PR exists, resolves commit SHAs, and calls the reusable workflow with `backend_sha`/`frontend_sha` override inputs. String override inputs are also available for exact SHAs not in the dropdown.

### Commit Choice Refresh
`refresh-preview-commits.yml` auto-updates the dispatch workflow's dropdown choices on every PR push event and merge to main. It fetches the 5 most recent commits from both repos' main branches and updates both the backend and frontend dispatch workflow YAMLs using `yq`. Requires `GHCR_PAT` with `workflow` scope.

### Secrets Resolution Order
The reusable workflow resolves secrets in this order:
1. Secrets passed from the **caller repo** (via `secrets: inherit`)
2. Secrets from the backend repo's **pr-preview environment**
3. Hardcoded **fallback defaults** in the workflow (e.g., `|| '1.0.0-beta1'`)

**Warning**: A stale secret at level 1 overrides levels 2-3. Always check caller repo secrets when debugging.
