#!/bin/bash
set -euo pipefail

# If an explicit tool is passed (migration/seed_db), run it directly
if [[ $# -gt 0 ]]; then
    case "$1" in
        migration|seed_db)
            exec /app/"$@"
            ;;
    esac
fi

# If no args passed (default case for migrator service), run SeaORM migration up
if [[ "$(basename "$0")" == "entrypoint.sh" ]]; then
    echo "🔧 Running SeaORM migration up (initial setup if needed)..."
    exec /app/migration up
fi

# Otherwise, start the main backend app
exec /app/refactor_platform_rs \
    -l "$BACKEND_LOG_FILTER_LEVEL" \
    -i "$BACKEND_INTERFACE" \
    -p "$BACKEND_PORT" \
    -d "$DATABASE_URL" \
    --allowed-origins="$BACKEND_ALLOWED_ORIGINS" \
    "$@"
