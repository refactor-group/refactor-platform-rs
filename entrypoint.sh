#!/bin/sh
set -euo pipefail

# determine the architecture of the host machine
ARCH=$(uname -m)

# select the binary based on the architecture
if [ "$ARCH" = "x86_64" ]; then
        echo "Executing AMD64 binary"
        exec /app/refactor_platform_rs \
            -l "$BACKEND_LOG_FILTER_LEVEL" \
            -i "$BACKEND_INTERFACE" \
            -p "$BACKEND_PORT" \
            -d "$DATABASE_URL" \
            --allowed-origins="$BACKEND_ALLOWED_ORIGINS" \
            "$@"
elif [ "$ARCH" = "aarch64" ]; then
        echo "Executing ARM64 binary"
        exec /app/refactor_platform_rs_arm64 \
            -l "$BACKEND_LOG_FILTER_LEVEL" \
            -i "$BACKEND_INTERFACE" \
            -p "$BACKEND_PORT" \
            -d "$DATABASE_URL" \
            --allowed-origins="$BACKEND_ALLOWED_ORIGINS" \
            "$@"
else
        echo "Unsupported architecture: $(uname -m)" >&2
        exit 1
fi
