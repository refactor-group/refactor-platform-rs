# syntax=docker/dockerfile:1

# Stage 1: Builder Stage
FROM --platform=$BUILDPLATFORM rust:latest AS builder
WORKDIR /usr/src/app

# Install dependencies
RUN apt-get update && apt-get install -y \
    bash \
    build-essential \
    pkg-config \
    libssl-dev \
    libpq-dev \
    perl \
    --no-install-recommends && \
    rm -rf /var/lib/apt/lists/*

# Conditional installation for ARM64
ARG TARGETPLATFORM
RUN if [ "$TARGETPLATFORM" = "linux/arm64" ]; then \
    apt-get update && apt-get install -y \
    gcc-aarch64-linux-gnu \
    libc6-dev-arm64-cross \
    --no-install-recommends && \
    rm -rf /var/lib/apt/lists/*; \
    fi

# Set environment variables for ARM64
ENV CARGO_TARGET_DIR=/usr/src/app/target
RUN if [ "$TARGETPLATFORM" = "linux/arm64" ]; then \
    rustup target add aarch64-unknown-linux-gnu && \
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc && \
    export RUSTFLAGS="-C link-arg=-L/usr/lib/aarch64-linux-gnu" && \
    export OPENSSL_STATIC=1 && \
    export CC=aarch64-linux-gnu-gcc && \
    export AR=aarch64-linux-gnu-ar && \
    export RANLIB=aarch64-linux-gnu-ranlib && \
    export OPENSSL_DIR=/usr/lib/aarch64-linux-gnu && \
    export OPENSSL_LIB_DIR=/usr/lib/aarch64-linux-gnu && \
    export OPENSSL_INCLUDE_DIR=/usr/include; \
    fi

# Copy source files
COPY Cargo.toml Cargo.lock ./
COPY ./entity/Cargo.toml ./entity/Cargo.toml
COPY ./entity_api/Cargo.toml ./entity_api/Cargo.toml
COPY ./migration/Cargo.toml ./migration/Cargo.toml
COPY ./service/Cargo.toml ./service/Cargo.toml
COPY ./web/Cargo.toml ./web/Cargo.toml
COPY . .

# Build the application
RUN cargo clean && \
    if [ "$TARGETPLATFORM" = "linux/arm64" ]; then \
    cargo build --release --target aarch64-unknown-linux-gnu --workspace; \
    else \
    cargo build --release --workspace; \
    fi

# Stage 2: Final Stage
FROM debian:stable-slim AS final
WORKDIR /app
COPY --from=builder /usr/src/app/target/release/refactor_platform_rs /app/refactor_platform_rs
COPY --from=builder /usr/src/app/target/release/migration /app/migration
COPY --from=builder /usr/src/app/target/release/seed_db /app/seed_db
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]
