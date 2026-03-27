#!/bin/bash
# /nginx/scripts/rotate-logs.sh
# Nginx log rotation script — archives access, error, and certbot logs into tar.bz3
#
# Rotation policy:
#   - Run every 6 months (via cron)
#   - Each run archives current logs into a timestamped tar.bz3
#   - Keeps the 2 most recent archives (covering ~1 year)
#   - Signals nginx to reopen log file descriptors after rotation
#
# Usage:
#   ./nginx/scripts/rotate-logs.sh
#
# Cron example (run on Jan 1 and Jul 1 at 2:00 AM):
#   0 2 1 1,7 * /path/to/refactor-platform-rs/nginx/scripts/rotate-logs.sh

set -euo pipefail

# ── Configuration ────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG_DIR="${SCRIPT_DIR}/../logs"
CONTAINER_NAME="nginx-reverse-proxy"
MAX_ARCHIVES=2  # Keep 2 archives × 6 months = 1 year of history

# ── Helpers ──────────────────────────────────────────────────────────
log() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $1"
}

die() {
    log "ERROR: $1"
    exit 1
}

# ── Pre-flight checks ───────────────────────────────────────────────
[ -d "$LOG_DIR" ] || die "Log directory does not exist: $LOG_DIR"

# Verify at least one log file exists and is non-empty
if [ ! -s "$LOG_DIR/access.log" ] && [ ! -s "$LOG_DIR/error.log" ] && [ ! -s "$LOG_DIR/letsencrypt-renewal.log" ]; then
    log "All log files are empty or missing — nothing to rotate"
    exit 0
fi

# Verify the nginx container is running (needed for log reopen signal)
if ! docker ps --filter "name=${CONTAINER_NAME}" --filter "status=running" --quiet | grep -q .; then
    die "Container ${CONTAINER_NAME} is not running — cannot signal nginx to reopen logs"
fi

# ── Rotate ───────────────────────────────────────────────────────────
TIMESTAMP="$(date '+%Y%m%d-%H%M%S')"
ARCHIVE_NAME="nginx-logs-${TIMESTAMP}.tar.bz3"

log "Starting log rotation"

# Move current logs to timestamped copies (nginx keeps writing to old fd)
cd "$LOG_DIR"

[ -f access.log ]              && mv access.log              "access-${TIMESTAMP}.log"
[ -f error.log ]               && mv error.log               "error-${TIMESTAMP}.log"
[ -f letsencrypt-renewal.log ] && mv letsencrypt-renewal.log "letsencrypt-renewal-${TIMESTAMP}.log"

# Create empty log files so nginx can reopen to them
touch access.log error.log letsencrypt-renewal.log

# Signal nginx to reopen log files (uses the new empty files)
log "Signaling nginx to reopen log files"
if ! docker exec "$CONTAINER_NAME" nginx -s reopen; then
    die "Failed to signal nginx — rotated logs are preserved as *-${TIMESTAMP}.log"
fi

# Collect rotated log files for archiving (only include files that exist)
ROTATED_FILES=()
[ -f "access-${TIMESTAMP}.log" ]              && ROTATED_FILES+=("access-${TIMESTAMP}.log")
[ -f "error-${TIMESTAMP}.log" ]               && ROTATED_FILES+=("error-${TIMESTAMP}.log")
[ -f "letsencrypt-renewal-${TIMESTAMP}.log" ] && ROTATED_FILES+=("letsencrypt-renewal-${TIMESTAMP}.log")

# Compress the rotated logs into a tar.bz3 archive
log "Compressing rotated logs into ${ARCHIVE_NAME}"
tar --use-compress-program=bzip3 -cf "$ARCHIVE_NAME" "${ROTATED_FILES[@]}"

# Remove the uncompressed rotated copies
rm -f "${ROTATED_FILES[@]}"

log "Archive created: ${LOG_DIR}/${ARCHIVE_NAME}"

# ── Prune old archives ──────────────────────────────────────────────
# List archives oldest-first, delete all but the newest MAX_ARCHIVES
ARCHIVE_COUNT=$(ls -1 nginx-logs-*.tar.bz3 2>/dev/null | wc -l | tr -d ' ')

if [ "$ARCHIVE_COUNT" -gt "$MAX_ARCHIVES" ]; then
    DELETE_COUNT=$((ARCHIVE_COUNT - MAX_ARCHIVES))
    log "Pruning ${DELETE_COUNT} old archive(s) (keeping newest ${MAX_ARCHIVES})"
    ls -1t nginx-logs-*.tar.bz3 | tail -n "$DELETE_COUNT" | while read -r old_archive; do
        log "  Deleting: ${old_archive}"
        rm -f "$old_archive"
    done
fi

log "Log rotation complete"
