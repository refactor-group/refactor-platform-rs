#!/bin/bash
set -euo pipefail

# Start the main Rust binary with runtime args/envs
exec /app/refactor_platform_rs \
    -l "$BACKEND_LOG_FILTER_LEVEL" \
    -i "$BACKEND_INTERFACE" \
    -p "$BACKEND_PORT" \
    -d "$DATABASE_URL" \
    --allowed-origins="$BACKEND_ALLOWED_ORIGINS" \
    "$@"
