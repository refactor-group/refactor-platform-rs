# syntax=docker/dockerfile:1

# Stage 1: Builder Stage for AMD64
FROM rust:latest AS builder-amd64
WORKDIR /usr/src/app

RUN apt-get update && apt-get install -y \
    bash \
    build-essential \
    pkg-config \
    libssl-dev \
    libpq-dev \
    perl \
    --no-install-recommends && \
    rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY ./entity/Cargo.toml ./entity/Cargo.toml
COPY ./entity_api/Cargo.toml ./entity_api/Cargo.toml
COPY ./migration/Cargo.toml ./migration/Cargo.toml
COPY ./service/Cargo.toml ./service/Cargo.toml
COPY ./web/Cargo.toml ./web/Cargo.toml
COPY . .

RUN cargo clean

# Set the Rust target directory
ENV CARGO_TARGET_DIR=/usr/src/app/target

RUN cargo build --release --workspace

# Stage 2: Builder Stage for ARM64
FROM rust:latest AS builder-arm64
WORKDIR /usr/src/app

RUN apt-get update && apt-get install -y \
    bash \
    build-essential \
    pkg-config \
    libssl-dev \
    libpq-dev \
    gcc-aarch64-linux-gnu \
    libc6-dev-arm64-cross \
    perl \
    --no-install-recommends && \
    rm -rf /var/lib/apt/lists/*

# Set Cargo linker and Rust flags for ARM64
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
ENV RUSTFLAGS="-C link-arg=-L/usr/lib/aarch64-linux-gnu"
ENV OPENSSL_STATIC=1

# Set the Rust target directory
ENV CARGO_TARGET_DIR=/usr/src/app/target

# Set environment variables for OpenSSL and cross-compilation
ENV CC=aarch64-linux-gnu-gcc
ENV AR=aarch64-linux-gnu-ar
ENV RANLIB=aarch64-linux-gnu-ranlib
ENV OPENSSL_DIR=/usr/lib/aarch64-linux-gnu
ENV OPENSSL_LIB_DIR=/usr/lib/aarch64-linux-gnu
ENV OPENSSL_INCLUDE_DIR=/usr/include


COPY Cargo.toml Cargo.lock ./
COPY ./entity/Cargo.toml ./entity/Cargo.toml
COPY ./entity_api/Cargo.toml ./entity_api/Cargo.toml
COPY ./migration/Cargo.toml ./migration/Cargo.toml
COPY ./service/Cargo.toml ./service/Cargo.toml
COPY ./web/Cargo.toml ./web/Cargo.toml
COPY . .

# Install cross for cross-compilation
RUN cargo clean && cargo install cross && \
    cross build --release --target aarch64-unknown-linux-gnu --workspace


# Stage 3: Merge the binaries
FROM debian:stable-slim AS merger
ARG TARGETPLATFORM
ARG TARGETARCH

WORKDIR /app

# target paths for AMD64
COPY --from=builder-amd64 /usr/src/app/target/release/refactor_platform_rs /app/refactor_platform_rs
COPY --from=builder-amd64 /usr/src/app/target/release/migration /app/migration
COPY --from=builder-amd64 /usr/src/app/target/release/seed_db /app/seed_db

# target paths for ARM64
COPY --from=builder-arm64 /usr/src/app/target/aarch64-unknown-linux-gnu/release/refactor_platform_rs /app/refactor_platform_rs_arm64
COPY --from=builder-arm64 /usr/src/app/target/aarch64-unknown-linux-gnu/release/migration /app/migration_arm64
COPY --from=builder-arm64 /usr/src/app/target/aarch64-unknown-linux-gnu/release/seed_db /app/seed_db_arm64

COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
