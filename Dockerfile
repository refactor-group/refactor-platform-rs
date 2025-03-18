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
    --no-install-recommends && \
    rm -rf /var/lib/apt/lists/*

# Set Cargo linker and Rust flags for ARM64
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
ENV RUSTFLAGS="-C link-arg=-L/usr/lib/aarch64-linux-gnu"

COPY Cargo.toml Cargo.lock ./
COPY ./entity/Cargo.toml ./entity/Cargo.toml
COPY ./entity_api/Cargo.toml ./entity_api/Cargo.toml
COPY ./migration/Cargo.toml ./migration/Cargo.toml
COPY ./service/Cargo.toml ./service/Cargo.toml
COPY ./web/Cargo.toml ./web/Cargo.toml
COPY . .

RUN cargo clean

RUN rustup target add aarch64-unknown-linux-gnu

# Set the Rust target directory
ENV CARGO_TARGET_DIR=/usr/src/app/target

RUN cargo build --release --workspace --target aarch64-unknown-linux-gnu

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
