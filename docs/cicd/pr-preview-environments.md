# PR Preview Environments - Developer Guide

## 🚀 Quick Start

**Want to test your changes in a live environment?** Just open a PR! A preview environment will be automatically deployed.

### What You Get

Every PR automatically gets:

- ✅ **Isolated full-stack environment** (Postgres + Backend + Frontend)
- ✅ **Clean path-based URLs** via NGINX routing
- ✅ **Live database** with migrations applied
- ✅ **Access via Tailscale VPN**
- ✅ **Automatic cleanup** when PR closes

### How to Access Your Preview

1. **Open a PR** in either `refactor-platform-rs` or `refactor-platform-fe`
2. **Wait for deployment** (~5-10 minutes for first build)
3. **Check PR comment** for your unique URLs
4. **Connect to Tailscale** VPN (required for access)
5. **Visit your preview** at the URLs provided

**Example PR Comment:**

```bash
🚀 PR Preview Environment Deployed!

Frontend:     http://neo.rove-barbel.ts.net/pr-201/
Backend API:  http://neo.rove-barbel.ts.net/pr-201/api/
Health Check: http://neo.rove-barbel.ts.net/pr-201/health
Base Path:    /pr-201/

Access Method: NGINX path-based routing (no direct port access)
```

---

## 🏗️ How It Works

### Path-Based Routing

Each PR gets a unique URL path based on the PR number:

| Service | URL Pattern | Example (PR #201) |
| --------- | ------------ | ------------------- |
| Frontend | `/pr-<NUM>/` | `http://neo.rove-barbel.ts.net/pr-201/` |
| Backend API | `/pr-<NUM>/api/` | `http://neo.rove-barbel.ts.net/pr-201/api/` |
| Health Check | `/pr-<NUM>/health` | `http://neo.rove-barbel.ts.net/pr-201/health` |

**How NGINX Routes Traffic:**

- All requests go through NGINX on port 80
- NGINX uses regex to match `/pr-<NUM>/` paths
- Routes to correct Docker containers based on PR number
- No application ports exposed to host

### Deployment Flow

**Backend PR:**

1. PR opened → Workflow triggers
2. Backend: Builds from **your PR branch** 📦
3. Frontend: Uses **main-arm64** image (or builds if missing)
4. Deploys: Full stack with your backend changes

**Frontend PR:**

1. PR opened → Workflow triggers
2. Frontend: Builds from **your PR branch** 📦
3. Backend: Uses **main-arm64** image (or builds if missing)
4. Deploys: Full stack with your frontend changes

### Architecture

```bash
┌──────────────────────────────────────────────┐
│  Developer (via Tailscale VPN)               │
└────────────────┬─────────────────────────────┘
                 │ HTTP
                 ↓
┌──────────────────────────────────────────────┐
│  NGINX (neo.rove-barbel.ts.net:80)           │
│  ├─ /pr-201/ → pr-201-frontend:3000          │
│  ├─ /pr-201/api/ → pr-201-backend:3000       │
│  └─ /pr-202/ → pr-202-frontend:3000          │
└────────────────┬─────────────────────────────┘
                 │ Docker Network: preview-ingress
                 ↓
┌──────────────────────────────────────────────┐
│  Docker Containers (No Host Ports)           │
│  ┌──────────────────────────────────────┐    │
│  │ PR-201 Environment                   │    │
│  │ ├─ pr-201-frontend-1 (3000)          │    │
│  │ ├─ pr-201-backend-1 (3000)           │    │
│  │ └─ pr-201-postgres-1 (5432, internal)│    │
│  └──────────────────────────────────────┘    │
│  ┌──────────────────────────────────────┐    │
│  │ PR-202 Environment                   │    │
│  │ ├─ pr-202-frontend-1 (3000)          │    │
│  │ ├─ pr-202-backend-1 (3000)           │    │
│  │ └─ pr-202-postgres-1 (5432, internal)│    │
│  └──────────────────────────────────────┘    │
└──────────────────────────────────────────────┘
```

**Security Features:**

- ✅ Single ingress point (NGINX only)
- ✅ No direct container port access
- ✅ Postgres never exposed externally
- ✅ Network isolation between PRs

---

## 🔧 Configuration

### Secrets & Variables

**All secrets are managed in ONE place:** Backend repo's `pr-preview` environment.

This means:

- ✅ Frontend repo needs **zero** PR preview secrets
- ✅ No secret duplication across repos
- ✅ Single source of truth for configuration

**Backend `pr-preview` Environment Contains:**

- RPi5 SSH connection details
- Database credentials
- TipTap API keys
- MailerSend API keys
- Frontend build configuration

### Workflow Files

**Backend Repository:**

- `.github/workflows/ci-deploy-pr-preview.yml` - Reusable workflow (does the heavy lifting)
- `.github/workflows/pr-preview-backend.yml` - Overlay for backend PRs

**Frontend Repository:**

- `.github/workflows/pr-preview-frontend.yml` - Overlay for frontend PRs (calls backend reusable workflow)

### NGINX Configuration

**Static Configuration:**

- `/etc/nginx/sites-enabled/pr-previews.conf` - Single config handles all PRs
- Uses regex to dynamically route based on PR number
- No per-PR config generation needed
- Automatic container discovery via Docker DNS

---

## 🧪 Testing Your Preview

### Health Check

```bash
# Check PR #201 health
curl http://neo.rove-barbel.ts.net/pr-201/health

# Expected response
PR #201 routing active
```

### API Testing

```bash
# List users endpoint (PR #201)
curl http://neo.rove-barbel.ts.net/pr-201/api/v1/users

# Create a test user
curl -X POST http://neo.rove-barbel.ts.net/pr-201/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","name":"Test User"}'
```

### Frontend Testing

Visit `http://neo.rove-barbel.ts.net/pr-201/` in your browser (Tailscale required).

### Database Access

Connect to your PR's database via SSH tunnel:

```bash
# SSH into Neo
ssh user@neo.rove-barbel.ts.net

# Access postgres container directly
docker exec -it pr-201-postgres-1 \
  psql -U refactor -d refactor_platform

# Or create SSH tunnel
ssh -L 5432:localhost:5432 user@neo.rove-barbel.ts.net

# Then connect from tunnel (note: postgres not exposed on host)
# You'll need to use docker exec approach above
```

---

## 🔍 Troubleshooting

### Deployment Failed

1. **Check workflow logs:**
   - Go to PR → "Checks" tab → Click on failed workflow
   - Review error messages in logs

2. **Common issues:**
   - **Linting errors:** Fix code formatting issues
   - **Test failures:** Ensure all tests pass locally first
   - **Build errors:** Check Dockerfile and dependencies
   - **Migration errors:** Verify database migrations are valid
   - **Image pull errors:** Check GHCR permissions and image exists

### Preview Not Accessible

1. **Verify Tailscale connection:**

   ```bash
   tailscale status
   # Should show you're connected to the network
   ```

2. **Check NGINX routing:**

   ```bash
   # Test NGINX health endpoint
   curl http://neo.rove-barbel.ts.net/health

   # Test PR-specific health
   curl http://neo.rove-barbel.ts.net/pr-201/health
   ```

3. **Verify containers running:**

   ```bash
   ssh user@neo.rove-barbel.ts.net
   docker ps --filter 'name=pr-201'
   ```

4. **Check container logs:**

   ```bash
   ssh user@neo.rove-barbel.ts.net
   docker logs pr-201-backend-1 --tail 50
   docker logs pr-201-frontend-1 --tail 50
   ```

### Environment Not Updating

- **Push new commits:** Workflow triggers on new commits
- **Re-run workflow:** Go to Actions → Re-run failed jobs
- **Check branch:** Ensure you're pushing to the PR branch
- **Verify build:** Check that new image was built and pushed

### NGINX Routing Issues

**502 Bad Gateway:**

- Container not running or name mismatch
- Check: `docker ps --filter 'name=pr-<NUM>'`
- Verify container names match pattern: `pr-<NUM>-frontend-1`, `pr-<NUM>-backend-1`

**404 Not Found:**

- Incorrect path in URL
- Ensure path starts with `/pr-<NUM>/`
- Check NGINX config: `cat /etc/nginx/sites-enabled/pr-previews.conf`

---

## 🧹 Cleanup

### Automatic Cleanup

Preview environments are **automatically cleaned up** when:

- PR is closed
- PR is merged

The cleanup workflow removes:

- Docker containers
- Database volumes
- Temporary files
- Compose and environment files

**Note:** NGINX config is static and shared across all PRs, so it's not removed during cleanup.

### Manual Cleanup (if needed)

If you need to manually clean up a preview:

```bash
# SSH into Neo
ssh user@neo.rove-barbel.ts.net

# Stop and remove PR environment
docker compose -p pr-201 down -v

# Remove compose and env files
rm ~/pr-201-compose.yaml ~/pr-201.env

# Verify cleanup
docker ps --filter 'name=pr-201'
# Should show no containers
```

---

## 🎯 Advanced Usage

### Force Rebuild

Trigger a complete rebuild (ignoring caches):

1. Go to Actions → CI Deploy PR Preview
2. Click "Run workflow"
3. Select your branch
4. Set `force_rebuild: true`

### Use Specific Image

Override backend or frontend image:

1. Edit overlay workflow (`.github/workflows/pr-preview-*.yml`)
2. Set `backend_image` or `frontend_image` input
3. Example: `backend_image: 'ghcr.io/refactor-group/refactor-platform-rs:main-arm64'`

### Test Different Branch Combinations

**Frontend PR using different backend branch:**

1. Edit `.github/workflows/pr-preview-frontend.yml`
2. Change `backend_branch: 'main'` to desired branch
3. Commit and push

**Backend PR using different frontend branch:**

1. Edit `.github/workflows/pr-preview-backend.yml`
2. Change `frontend_branch: 'main'` to desired branch
3. Commit and push

### Inspect NGINX Configuration

```bash
# SSH into Neo
ssh user@neo.rove-barbel.ts.net

# View NGINX config
cat /etc/nginx/sites-enabled/pr-previews.conf

# Test NGINX configuration syntax
sudo nginx -t

# View NGINX access logs
sudo tail -f /var/log/nginx/access.log | grep 'pr-'

# View NGINX error logs
sudo tail -f /var/log/nginx/error.log | grep 'pr-'
```

---

## 📊 Monitoring

### View Logs

**Real-time logs during deployment:**

```bash
# SSH into Neo
ssh user@neo.rove-barbel.ts.net

# View backend logs
docker logs pr-201-backend-1 -f

# View frontend logs
docker logs pr-201-frontend-1 -f

# View postgres logs
docker logs pr-201-postgres-1 -f

# View migration logs
docker logs pr-201-migrator-1
```

### Check Container Status

```bash
# SSH into Neo
ssh user@neo.rove-barbel.ts.net

# List all containers for your PR
docker compose -p pr-201 ps

# View resource usage
docker stats pr-201-backend-1 pr-201-frontend-1 pr-201-postgres-1

# Check container networks
docker inspect pr-201-backend-1 --format='{{range $k := .NetworkSettings.Networks}}{{printf "%s\n" $k}}{{end}}'
# Expected: pr-201_default, preview-ingress
```

### Network Verification

```bash
# Check preview-ingress network
docker network inspect preview-ingress

# Verify NGINX can reach containers
docker run --rm --network preview-ingress alpine ping -c 1 pr-201-backend-1
# Should succeed

# Verify postgres is NOT on preview-ingress (security)
docker run --rm --network preview-ingress alpine ping -c 1 pr-201-postgres-1
# Should fail (postgres only on pr-201_default network)
```

---

## 🔐 Security Notes

- **Tailscale VPN Required:** Previews are not publicly accessible
- **NGINX Single Ingress:** All traffic goes through NGINX (port 80 only)
- **No Direct Port Access:** Application containers don't expose host ports
- **Postgres Isolation:** Database never accessible from preview-ingress network
- **Network Isolation:** Each PR has isolated Docker network for internal communication
- **Shared Environment:** All PRs deploy to same Neo server (isolated by Docker networks)
- **Temporary Data:** Database resets when environment is cleaned up
- **Do Not:** Store sensitive production data in preview environments

---

## 🤝 Contributing to PR Preview System

Want to improve the PR preview system?

**Key files to modify:**

- `ci-deploy-pr-preview.yml` - Main deployment logic
- `docker-compose.pr-preview.yaml` - Service definitions and network configuration
- `pr-preview-backend.yml` / `pr-preview-frontend.yml` - Trigger configurations
- `nginx/conf.d/pr-previews.conf` - NGINX routing configuration (static, handles all PRs)

**After changes:**

1. Test in a PR first
2. Document changes in this runbook
3. Update PR template if user-facing changes

**NGINX Configuration Changes:**

- The NGINX config is static and must be manually updated on Neo if modified
- Location: `/etc/nginx/sites-enabled/pr-previews.conf`
- After updating: `sudo nginx -t && sudo nginx -s reload`

---

## 📚 Additional Resources

- [GitHub Actions Workflow Syntax](https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions)
- [Docker Compose Documentation](https://docs.docker.com/compose/)
- [Docker Networking](https://docs.docker.com/network/)
- [NGINX Configuration](https://nginx.org/en/docs/)
- [Tailscale Setup Guide](https://tailscale.com/kb/start/)

---

**Questions?** Ask in Levi in Slack or open an issue.
