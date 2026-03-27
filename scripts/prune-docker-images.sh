#!/bin/bash
# /scripts/prune-docker-images.sh
# Removes unused Docker images older than 30 days to reclaim disk space.
#
# Pruning policy:
#   - Run monthly (via cron)
#   - Removes all images not used by a running container that are older than 30 days
#   - Also prunes dangling build cache
#
# Usage:
#   ./scripts/prune-docker-images.sh
#
# Cron example (run on the 1st of every month at 3:00 AM):
#   0 3 1 * * /path/to/refactor-platform-rs/scripts/prune-docker-images.sh

set -euo pipefail

# ── Helpers ──────────────────────────────────────────────────────────
log() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $1"
}

# ── Disk usage before ────────────────────────────────────────────────
log "Docker disk usage before pruning:"
docker system df

# ── Prune unused images older than 30 days ───────────────────────────
log "Pruning unused images older than 30 days"
docker image prune -a -f --filter "until=720h"

# ── Prune dangling build cache ───────────────────────────────────────
log "Pruning dangling build cache"
docker builder prune -f --filter "until=720h"

# ── Disk usage after ─────────────────────────────────────────────────
log "Docker disk usage after pruning:"
docker system df

log "Docker image pruning complete"
