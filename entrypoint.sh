#!/bin/bash
set -euo pipefail                                      # strict mode

# ROLE defines what to run: migrator or app server
ROLE="${ROLE:-app}"                                    # defaults to app server

# If explicitly calls a helper (e.g. `migration status`)
if [[ $# -gt 0 ]]; then                                # check for CLI args
  case "$1" in
    migration|seed_db) exec "/app/$@" ;;               # hand-off to migrator
  esac
fi

# if ROLE is migrator, run migrations
if [[ "$ROLE" == "migrator" ]]; then
  echo "🔧 Running SeaORM migrate up…"
  exec /app/migration up                               # exits 0 if nothing to do
fi

# default to start API server
exec /app/refactor_platform_rs \
  -l "${BACKEND_LOG_FILTER_LEVEL:-info}" \
  -i "${BACKEND_INTERFACE:-0.0.0.0}" \
  -p "${BACKEND_PORT:-8080}" \
  -d "${DATABASE_URL}" \
  --allowed-origins="${BACKEND_ALLOWED_ORIGINS:-*}" \
  "$@"