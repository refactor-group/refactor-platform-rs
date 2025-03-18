#!/bin/sh
set -euo pipefail

# Determine the architecture of the host machine
ARCH=$(uname -m)

# Set Rust linker for ARM64 if needed
if [ "$ARCH" = "aarch64" ]; then
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
    export RUSTFLAGS="-C link-arg=-L/usr/lib/aarch64-linux-gnu"
fi

# Select the binary based on the architecture
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
