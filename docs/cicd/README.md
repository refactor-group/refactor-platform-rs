# CI/CD Infrastructure Documentation

**Last Updated:** 2025-12-29
**Version:** 1.0.0-beta2
**Maintainer:** DevOps Team

## Overview

The Refactor Platform backend uses GitHub Actions for continuous integration, release builds, and production deployment. This document provides a comprehensive overview of the CI/CD infrastructure.

**Quick Stats:**
- **3 GitHub Actions Workflows** (CI, Release, Deploy)
- **Docker-based Deployment** to GitHub Container Registry (GHCR)
- **Production Platform:** DigitalOcean (accessed via Tailscale VPN)
- **Container Orchestration:** Docker Compose with Nginx reverse proxy
- **Database:** PostgreSQL with SeaORM migrations

---

## Table of Contents

1. [GitHub Actions Workflows](#github-actions-workflows)
2. [Docker Infrastructure](#docker-infrastructure)
3. [Database Migrations](#database-migrations)
4. [Release Process](#release-process)
5. [Production Deployment](#production-deployment)
6. [Security & Secrets](#security--secrets)
7. [Quick Reference](#quick-reference)
8. [Gap Analysis & Future Improvements](#gap-analysis--future-improvements)

---

## GitHub Actions Workflows

### 1. Branch CI Pipeline
**File:** `.github/workflows/build-test-push.yml`
**Triggers:** Push to main, Pull requests to main, Manual dispatch
**Documentation:** [workflows/build-test-push.md](workflows/build-test-push.md)

**Jobs:**
1. **Lint** - Clippy (zero warnings) + rustfmt check
2. **Test** - Build all targets + run test suite
3. **Docker** - Build and push images to GHCR (only if lint and test pass)

**Image Tags:**
- `ghcr.io/refactor-group/refactor-platform-rs/{branch}:latest`
- `ghcr.io/refactor-group/refactor-platform-rs/{branch}:{git-sha}`

**Key Features:**
- ✅ Quality gates (lint/test before docker build)
- ✅ Rust dependency caching (Swatinem/rust-cache)
- ✅ Docker layer caching (GitHub Actions cache)
- ✅ Build provenance attestations (main branch only)

### 2. Production Release Builds
**File:** `.github/workflows/build_and_push_production_images.yml`
**Triggers:** GitHub releases (type: released), Manual dispatch
**Documentation:** [workflows/production-images.md](workflows/production-images.md)

**Multi-Architecture Builds:**
- linux/amd64
- linux/arm64

**Image Tag:** `ghcr.io/refactor-group/refactor-platform-rs:stable`

**Key Features:**
- ✅ Runs full test suite before building
- ✅ Multi-platform builds for broader deployment options
- ✅ Tagged as "stable" for production use
- ✅ Build provenance attestations

### 3. Production Deployment
**File:** `.github/workflows/deploy_to_do.yml`
**Triggers:** Manual dispatch only
**Documentation:** [workflows/deploy-to-do.md](workflows/deploy-to-do.md)

**Deployment Flow:**
1. Establish Tailscale VPN connection to DigitalOcean server
2. Generate `.env` file from GitHub secrets/variables
3. Download docker-compose.yaml and nginx configs
4. Stop systemd service (`refactor-platform.service`)
5. Pull latest Docker images from GHCR
6. Start systemd service
7. Verify deployment (health checks, container status)

**Key Features:**
- ✅ Secure VPN-based deployment (Tailscale)
- ✅ Systemd service management
- ✅ Health checks and verification
- ✅ Deploys both backend and frontend together
- ⚠️ Manual trigger only (no auto-deploy)

---

## Docker Infrastructure

### Multi-Stage Dockerfile
**File:** `Dockerfile`
**Documentation:** [docker/dockerfile-guide.md](docker/dockerfile-guide.md)

**Stages:**
1. **Chef (Planner)** - Uses cargo-chef to analyze dependencies
2. **Builder** - Compiles dependencies and application separately for optimal caching
3. **Runtime** - Minimal Debian Bullseye Slim with only necessary binaries

**Compiled Binaries:**
- `refactor_platform_rs` - Main API server
- `migration` - Database migration tool (migrationctl)

**Security Features:**
- Non-root user (appuser, UID 1001)
- Minimal runtime dependencies
- Only essential files copied to runtime image

### Docker Compose
**Files:**
- `docker-compose.yaml` - Production configuration
- `docker-compose.dev-staging.yaml` - Dev/staging overlay (adds PostgreSQL container)

**Documentation:** [docker/docker-compose-guide.md](docker/docker-compose-guide.md)

**Services:**
| Service | Purpose | Restart Policy |
|---------|---------|----------------|
| nginx | Reverse proxy (ports 80, 443) | unless-stopped |
| migrator | Runs database migrations once | no (exits after completion) |
| rust-app | Backend API (port 4000) | unless-stopped |
| nextjs-app | Frontend (port 3000) | unless-stopped |
| postgres | Database (dev/staging only) | unless-stopped |

**Network:** `backend_network` (bridge mode)

### Entrypoint Script
**File:** `entrypoint.sh`
**Documentation:** [docker/entrypoint-guide.md](docker/entrypoint-guide.md)

**Execution Modes (via `ROLE` environment variable):**
- `ROLE=migrator` - Runs database migrations via SeaORM CLI
- `ROLE=app` - Runs the Rust API server

**Features:**
- Environment variable validation
- Comprehensive logging
- Configuration defaults for common settings

---

## Database Migrations

**Framework:** SeaORM Migration CLI
**Total Migrations:** 15 (1 base SQL + 14 SeaORM migrations)
**Documentation:** [database/migration-workflow.md](database/migration-workflow.md)

**Migration Execution:**
- Automatically runs in `migrator` container before app startup
- Uses custom PostgreSQL schema: `refactor_platform`
- Database user: `refactor`

**⚠️ CRITICAL: PostgreSQL Type Ownership**

When creating PostgreSQL types with `create_type()`, you MUST immediately follow with:
```sql
ALTER TYPE refactor_platform.<type_name> OWNER TO refactor;
```

This prevents "must be owner of type" errors in subsequent migrations.
**See:** [database/postgresql-type-ownership.md](database/postgresql-type-ownership.md)

**Local Development Scripts:**
- `scripts/rebuild_db.sh` - Local PostgreSQL setup
- `scripts/rebuild_db_container.sh` - Containerized setup

---

## Release Process

**Current Status:** Manual process
**Documentation:** [releases/release-workflow.md](releases/release-workflow.md)

**Release Steps:**
1. **Version Bump** - Manually edit `Cargo.toml` (all workspace members at same version)
2. **Release Notes** - Create markdown file (e.g., `RELEASE_NOTES_1.0.0-beta1.md`)
3. **GitHub Release** - Create release through GitHub UI
   - Triggers `build_and_push_production_images.yml`
   - Builds multi-arch images tagged as "stable"
4. **Deployment** - Manually trigger `deploy_to_do.yml` workflow

**Frontend Coordination:**
- Frontend repo: `../refactor-platform-fe`
- Both currently at version 1.0.0-beta2
- Versions appear to be manually synchronized
- Both deployed together in same workflow

**See:** [releases/frontend-backend-coordination.md](releases/frontend-backend-coordination.md)

---

## Production Deployment

**Platform:** DigitalOcean Droplet
**Access:** Tailscale VPN (private network)
**Service Management:** systemd (`refactor-platform.service`)
**Reverse Proxy:** Nginx with Let's Encrypt SSL

**Domains:**
- myrefactor.com (primary)
- refactor.engineer (redirects to primary)

**SSL Certificates:**
- Let's Encrypt certificates
- Auto-renewal script: `nginx/scripts/renew-certs.sh`

**Database:**
- PostgreSQL (appears to be DigitalOcean managed service)
- SSL/TLS connection with certificate verification

**Systemd Commands:**
```bash
# View status
systemctl status refactor-platform.service

# View logs
journalctl -xeu refactor-platform.service -f

# Restart service
systemctl restart refactor-platform.service
```

---

## Security & Secrets

**Documentation:** [security/secrets-management.md](security/secrets-management.md)

**GitHub Secrets Required:** (30+ secrets and variables)

**Categories:**
- **DigitalOcean:** SSH keys, host keys, server details
- **Tailscale:** OAuth client credentials
- **Database:** User, password, SSL certificate
- **Container Registry:** GHCR credentials
- **External Services:** TipTap, MailerSend API keys

**VPN Access:**
- **Tailscale VPN** provides secure access to production server
- OAuth-based authentication
- **Documentation:** [security/tailscale-vpn.md](security/tailscale-vpn.md)

---

## Quick Reference

### Running Workflows

**Branch Build:**
```bash
# Automatically triggered on:
git push origin <branch-name>

# Or manual trigger via GitHub Actions UI
```

**Create Release:**
1. Go to GitHub > Releases
2. Click "Create new release"
3. Choose tag (e.g., `1.0.0-beta3`)
4. Write release notes
5. Publish release → Triggers production image build

**Deploy to Production:**
1. Go to GitHub Actions
2. Select "Deploy to DigitalOcean" workflow
3. Click "Run workflow"
4. Optionally enable SSH debugging
5. Confirm and run

### Docker Commands

**Build Locally:**
```bash
docker build -t refactor-backend .
```

**Run Locally (app mode):**
```bash
docker run --rm --env-file .env -p 4000:4000 refactor-backend
```

**Run Locally (migrator mode):**
```bash
docker run --rm -e ROLE=migrator -e DATABASE_URL=... refactor-backend
```

**Docker Compose (Dev/Staging):**
```bash
# Start with local PostgreSQL
docker compose -f docker-compose.yaml -f docker-compose.dev-staging.yaml up

# View logs
docker compose logs -f rust-app

# Stop all services
docker compose down

# Remove volumes too
docker compose down -v
```

### Migration Commands

**Create New Migration:**
```bash
cd migration
sea-orm-cli migrate generate <migration_name>
```

**Run Migrations (Local):**
```bash
DATABASE_URL=postgres://refactor:password@localhost:5432/refactor_platform \
sea-orm-cli migrate up -s refactor_platform
```

**Refresh Database (Local):**
```bash
./scripts/rebuild_db.sh
```

---

## Gap Analysis & Future Improvements

### Current Gaps

**Documentation Gaps:** ✅ Being addressed in this PR

**Infrastructure Gaps:**
- ⚠️ No automated semantic versioning
- ⚠️ No automated changelog generation
- ⚠️ No security scanning (cargo-audit, Dependabot)
- ⚠️ No test coverage reporting
- ⚠️ No deployment notifications
- ⚠️ No staging environment
- ⚠️ No automated rollback mechanism
- ⚠️ Manual production deployment (human error risk)

**See Complete Analysis:** [gap-analysis.md](gap-analysis.md)

### Improvement Roadmap

**High Priority (Next):**
1. Add Dependabot for automated dependency updates
2. Add cargo-audit security scanning to CI
3. Implement deployment notifications (Slack/email)
4. Create deployment checklists
5. Add test coverage reporting

**Medium Priority:**
1. Implement semantic versioning automation
2. Add staging environment
3. Automated changelog generation
4. Infrastructure as code (Terraform)

**See Complete Roadmap:** [future-improvements.md](future-improvements.md)

---

## Additional Resources

- [Workflow Documentation](workflows/) - Detailed workflow analysis
- [Docker Documentation](docker/) - Container and orchestration guides
- [Database Documentation](database/) - Migration and setup guides
- [Release Documentation](releases/) - Release process and coordination
- [Security Documentation](security/) - Secrets, VPN, and SSL management
- [Diagrams](diagrams/) - Visual representations of CI/CD flows
- [Templates](templates/) - Checklists and runbook templates

---

## Maintenance

This documentation should be updated when:
- New workflows are added or existing ones significantly modified
- Docker infrastructure changes (new services, configuration changes)
- Database migration process changes
- Release process changes
- New secrets or environment variables are added
- Production infrastructure changes

**Documentation Owner:** DevOps Team
**Review Frequency:** Quarterly or with major changes
**Last Review:** 2025-12-29
