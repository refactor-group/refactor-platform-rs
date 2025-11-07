# PR Preview Environments - Developer Guide

## 🚀 Quick Start

**Want to test your changes in a live environment?** Just open a PR! A preview environment will be automatically deployed.

### What You Get

Every PR automatically gets:
- ✅ **Isolated full-stack environment** (Postgres + Backend + Frontend)
- ✅ **Unique ports** based on your PR number
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
```
🚀 PR Preview Environment Deployed!

Frontend: http://rpi5-hostname:3042
Backend:  http://rpi5-hostname:4042
Health:   http://rpi5-hostname:4042/health

Ports: Frontend: 3042 | Backend: 4042 | Postgres: 5474
```

---

## 🏗️ How It Works

### Port Allocation

Each PR gets unique ports calculated from the PR number:

| Service | Formula | Example (PR #42) |
|---------|---------|------------------|
| Frontend | 3000 + PR# | 3042 |
| Backend | 4000 + PR# | 4042 |
| Postgres | 5432 + PR# | 5474 |

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

```
┌─────────────────────────────────────────────────┐
│  GitHub Actions Workflow                        │
│  ├─ Lint & Test                                │
│  ├─ Build ARM64 Images (on Neo runner)         │
│  └─ Deploy to RPi5 via Tailscale SSH           │
└─────────────────────────────────────────────────┘
                    ↓
┌─────────────────────────────────────────────────┐
│  RPi5 (ARM64) - Preview Environment            │
│  ├─ Postgres (port: 5432 + PR#)                │
│  ├─ Backend  (port: 4000 + PR#)                │
│  └─ Frontend (port: 3000 + PR#)                │
└─────────────────────────────────────────────────┘
```

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

---

## 🧪 Testing Your Preview

### Health Check

```bash
# Check backend health
curl http://rpi5-hostname:4042/health

# Expected response
{"status":"ok"}
```

### API Testing

```bash
# List users endpoint
curl http://rpi5-hostname:4042/api/v1/users

# Create a test user (if endpoint exists)
curl -X POST http://rpi5-hostname:4042/api/v1/users \
  -H "Content-Type: application/json" \
  -d '{"email":"test@example.com","name":"Test User"}'
```

### Database Access

Connect to your PR's database:

```bash
# SSH tunnel to Postgres
ssh -L 5432:localhost:5474 user@rpi5-hostname

# Then connect locally
psql -h localhost -p 5432 -U refactor -d refactor
```

### Frontend Testing

Visit `http://rpi5-hostname:3042` in your browser (Tailscale required).

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

### Preview Not Accessible

1. **Verify Tailscale connection:**
   ```bash
   tailscale status
   # Should show you're connected to the network
   ```

2. **Check service status:**
   - View PR comment for deployment status
   - Check workflow logs for errors

3. **Verify ports:**
   - Ensure you're using the correct port from PR comment
   - Ports are unique per PR (3000+PR#, 4000+PR#)

### Environment Not Updating

- **Push new commits:** Workflow triggers on new commits
- **Re-run workflow:** Go to Actions → Re-run failed jobs
- **Check branch:** Ensure you're pushing to the PR branch

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

### Manual Cleanup (if needed)

If you need to manually clean up a preview:

```bash
# SSH into RPi5
ssh user@rpi5-hostname

# Stop and remove PR environment
docker compose -p pr-42 down -v

# Remove compose file
rm ~/pr-42-compose.yaml ~/pr-42.env
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

---

## 📊 Monitoring

### View Logs

**Real-time logs during deployment:**
```bash
# SSH into RPi5
ssh user@rpi5-hostname

# View backend logs
docker logs pr-42-backend-1 -f

# View frontend logs
docker logs pr-42-frontend-1 -f

# View postgres logs
docker logs pr-42-postgres-1 -f

# View migration logs
docker logs pr-42-migrator-1
```

### Check Container Status

```bash
# SSH into RPi5
ssh user@rpi5-hostname

# List all containers for your PR
docker compose -p pr-42 ps

# View resource usage
docker stats pr-42-backend-1 pr-42-frontend-1 pr-42-postgres-1
```

---

## 🔐 Security Notes

- **Tailscale VPN Required:** Previews are not publicly accessible
- **Shared Environment:** All PRs deploy to same RPi5 (isolated by Docker Compose projects)
- **Temporary Data:** Database resets when environment is cleaned up
- **Do Not:** Store sensitive production data in preview environments

---

## 🤝 Contributing to PR Preview System

Want to improve the PR preview system?

**Key files to modify:**
- `ci-deploy-pr-preview.yml` - Main deployment logic
- `docker-compose.pr-preview.yaml` - Service definitions
- `pr-preview-backend.yml` / `pr-preview-frontend.yml` - Trigger configurations

**After changes:**
1. Test in a PR first
2. Document changes in this runbook
3. Update PR template if user-facing changes

---

## 📚 Additional Resources

- [GitHub Actions Workflow Syntax](https://docs.github.com/en/actions/using-workflows/workflow-syntax-for-github-actions)
- [Docker Compose Documentation](https://docs.docker.com/compose/)
- [Tailscale Setup Guide](https://tailscale.com/kb/start/)

---

**Questions?** Ask in #engineering Slack channel or open an issue.

**Happy Testing! 🚀**
