#!/bin/sh
set -euo pipefail

case "$(uname -m)" in
    x86_64)
        echo "Executing AMD64 binary"
        exec /usr/src/app/target/x86_64-unknown-linux-gnu/release/refactor_platform_rs \
            -l "$BACKEND_LOG_FILTER_LEVEL" \
            -i "$BACKEND_INTERFACE" \
            -p "$BACKEND_PORT" \
            -d "$DATABASE_URL" \
            --allowed-origins="$BACKEND_ALLOWED_ORIGINS" \
            "$@"
        ;;
    aarch64)
        echo "Executing ARM64 binary"
        exec /usr/src/app/target/aarch64-unknown-linux-gnu/release/refactor_platform_rs \
            -l "$BACKEND_LOG_FILTER_LEVEL" \
            -i "$BACKEND_INTERFACE" \
            -p "$BACKEND_PORT" \
            -d "$DATABASE_URL" \
            --allowed-origins="$BACKEND_ALLOWED_ORIGINS"\
            "$@"
        ;;
    *)
        echo "Unsupported architecture: $(uname -m)" >&2
        exit 1
        ;;
esac