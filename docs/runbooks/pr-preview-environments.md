# PR Preview Environments

Automated isolated staging environments for every pull request.

---

## 🚀 Quick Start

1. **Create PR** to `main` branch
2. **Wait 5-10 min** for deployment
3. **Connect to Tailscale** VPN
4. **Click backend URL** in PR comment
5. **Test your changes**

Cleanup happens automatically when PR closes/merges.

---

## 💡 What & Why

### The Problem
- Manual deployment for testing
- Environment conflicts between developers
- Changes merged without full-stack testing
- Slow feedback loops

### The Solution
**Automatic isolated environments** that deploy on every PR:
- ✅ Own database, network, and ports
- ✅ Run 10+ PRs simultaneously
- ✅ Auto-cleanup on close/merge
- ✅ Live in 5-10 minutes
- ✅ Warm build cache for fast deployments

---

## 🏗️ How It Works

```
PR opened/updated
  → GitHub Actions builds image (uses warm cache)
  → Deploys to RPi5 via Tailscale
  → Bot comments with URLs
  → Test via Tailscale
  → PR closes → Auto cleanup

Nightly (3 AM UTC)
  → Cache warming builds ARM64 from main
  → PR builds start with warm cache
```

**Each PR gets:**
- Postgres container (fresh DB with migrations)
- Backend API container (your PR code)
- Isolated Docker network
- Unique ports (no conflicts)

---

## 🔌 Accessing Your Environment

### Prerequisites
- Tailscale installed and connected
- Member of team Tailscale network

### Access Steps

**1. Find your preview URL in PR comment:**
```markdown
🚀 PR Preview Environment Deployed!
Backend API: http://neo.rove-barbel.ts.net:4123
Health Check: http://neo.rove-barbel.ts.net:4123/health
```

**2. Connect to Tailscale:**
```bash
tailscale status  # Verify connected
```

**3. Click URLs** (only works on Tailscale!)

---

## 🧮 Port Allocation

**Formula:**
```
Backend Port  = 4000 + PR_NUMBER
Postgres Port = 5432 + PR_NUMBER
```

**Examples:**
- PR #1 → Backend: `4001`, Postgres: `5433`
- PR #123 → Backend: `4123`, Postgres: `5555`
- PR #999 → Backend: `4999`, Postgres: `6431`

---

## 🧪 Testing Your Changes

### Health Check
```bash
curl http://neo.rove-barbel.ts.net:4123/health
```

### API Testing
```bash
PR_NUM=123
BASE_URL="http://neo.rove-barbel.ts.net:$((4000 + PR_NUM))"

curl $BASE_URL/api/v1/users
curl $BASE_URL/health
```

### Database Access
```bash
psql -h neo.rove-barbel.ts.net -p 5555 -U refactor -d refactor
```

### Browser
Open while connected to Tailscale:
```
http://neo.rove-barbel.ts.net:4123/health
```

---

## 🔧 Troubleshooting

### ❌ Can't Access URL

**Check Tailscale:**
```bash
tailscale status | grep neo
```

**Verify container running:**
```bash
ssh deploy@neo.rove-barbel.ts.net 'docker ps | grep pr-123'
```

**Check deployment succeeded:**
- Go to PR → Checks tab → Look for green checkmark

### ❌ Deployment Failed

**View logs:** PR → Checks tab → Click failed step

**Common issues:**
- Build errors → Check Rust compilation logs
- SSH timeout → Verify Tailscale OAuth in GitHub secrets
- Container won't start → Check backend logs on RPi5

### ❌ Slow Deployment (10+ min)

**Normal times:**
- **First PR after midnight:** 5-10 min (cache warmed nightly)
- **Subsequent PRs:** 3-5 min (using cache)
- **Cache miss:** 15-20 min (full rebuild)

**If unexpectedly slow:**
- Cache corruption → Check nightly cache warming workflow
- Build complexity → Large code changes take longer
- RPi5 load → Multiple simultaneous builds

**Verify cache warming:**
```bash
# Check nightly workflow ran successfully
GitHub → Actions → "Warm Build Cache" → Latest run
```

### 🔍 View Container Logs

```bash
ssh deploy@neo.rove-barbel.ts.net

# Backend logs
docker logs pr-123-backend-1 --tail 50

# Migration logs
docker logs pr-123-migrator-1

# All PR containers
docker ps --filter "name=pr-"
```

---

## ⚙️ Configuration

### Update Environment Variables

**Location:** `Settings → Environments → pr-preview`

**Common changes:**
- `BACKEND_LOG_LEVEL`: `DEBUG` → `INFO`
- `BACKEND_SESSION_EXPIRY`: `86400` (24h) → `3600` (1h)

### Add New Environment Variable

**1. Add to GitHub:** `Settings → Environments → pr-preview → Add secret`

**2. Add to workflow:**
```yaml
env:
  MY_VAR: ${{ secrets.MY_VAR }}
```

**3. Add to SSH export in deployment step:**
```bash
export MY_VAR='${MY_VAR}'
```

**4. Add to `docker-compose.pr-preview.yaml`:**
```yaml
environment:
  MY_VAR: ${MY_VAR}
```

---

## 🧹 Cleanup Behavior

**Automatic cleanup when PR closes:**
- ✅ Containers stopped and removed
- ✅ Docker images deleted
- ✅ Networks removed
- ✅ Compose files deleted

**Volume retention:**
- **Merged PRs:** 7-day retention (allows post-merge debugging)
- **Closed PRs:** Immediate removal (frees space)

**Manual cleanup (if needed):**
```bash
ssh deploy@neo.rove-barbel.ts.net
docker compose -p pr-123 -f pr-123-compose.yaml down
docker volume rm pr-123_postgres_data
```

---

## 🎯 Manual Deployment (No PR)

**Use workflow dispatch:**
1. Actions tab → "Deploy PR Preview to RPi5"
2. Click "Run workflow"
3. Select branch and options
4. Click "Run workflow"

**Note:** No PR comment (no PR to comment on)

---

## ❓ FAQ

**Q: How many PRs can run simultaneously?**  
A: ~10-15 comfortably on RPi5

**Q: What if deployment fails?**  
A: PR still mergeable, check workflow logs for errors

**Q: Can I test frontend changes?**  
A: Not yet, backend only (frontend coming later)

**Q: How do I see active environments?**
```bash
ssh <username>@neo.rove-barbel.ts.net 'docker ps --filter "name=pr-"'
```

**Q: Why is my first PR build slow?**  
A: Cache warming runs nightly at 3 AM UTC. PRs before first cache warm take 15-20min.

**Q: Where are the workflows?**  
A: `.github/workflows/deploy-pr-preview.yml` (deploy)  
A: `.github/workflows/cleanup-pr-preview.yml` (cleanup)  
A: `.github/workflows/warm-build-cache.yml` (nightly cache)

---

## 📁 Key Files

| File | Purpose |
|------|---------|
| `.github/workflows/deploy-pr-preview.yml` | Deployment automation |
| `.github/workflows/cleanup-pr-preview.yml` | Cleanup automation |
| `.github/workflows/warm-build-cache.yml` | Nightly cache warming |
| `docker-compose.pr-preview.yaml` | Multi-tenant template |

---

## 🆘 Getting Help

1. Check troubleshooting section above
2. Review GitHub Actions logs
3. SSH to RPi5 and check container logs
4. Ask in `Levi` Slack

---

**Last Updated:** 2025-11-02  
**Maintained By:** Platform Engineering Team (aka Levi)
