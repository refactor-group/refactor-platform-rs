# syntax=docker/dockerfile:1
# Specify the Dockerfile syntax version

# Stage 1: Builder Stage for AMD64
FROM rust:latest AS builder-amd64

# Set the working directory inside the container
WORKDIR /usr/src/app

# Install necessary packages for building Rust projects with PostgreSQL dependencies
RUN apt-get update && apt-get install -y \
    bash \
    build-essential \
    pkg-config \
    libssl-dev \
    libpq-dev \
    --no-install-recommends && \
    rm -rf /var/lib/apt/lists/*

# Copy the main workspace Cargo.toml and Cargo.lock to define workspace structure
COPY Cargo.toml Cargo.lock ./

# Copy each module's Cargo.toml to maintain the workspace structure
COPY ./entity/Cargo.toml ./entity/Cargo.toml
COPY ./entity_api/Cargo.toml ./entity_api/Cargo.toml
COPY ./migration/Cargo.toml ./migration/Cargo.toml
COPY ./service/Cargo.toml ./service/Cargo.toml
COPY ./web/Cargo.toml ./web/Cargo.toml

# Copy the complete source code into the container's working directory
COPY . .

# Remove the target directory to ensure a clean build.
RUN cargo clean

# Build the Rust application in release mode for the AMD64 target
RUN cargo build --release --workspace

# Stage 2: Builder Stage for ARM64
FROM rust:latest AS builder-arm64

# Set the working directory inside the container
WORKDIR /usr/src/app

# Install necessary packages for building Rust projects with PostgreSQL dependencies
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

# Copy the main workspace Cargo.toml and Cargo.lock to define workspace structure
COPY Cargo.toml Cargo.lock ./

# Copy each module's Cargo.toml to maintain the workspace structure
COPY ./entity/Cargo.toml ./entity/Cargo.toml
COPY ./entity_api/Cargo.toml ./entity_api/Cargo.toml
COPY ./migration/Cargo.toml ./migration/Cargo.toml
COPY ./service/Cargo.toml ./service/Cargo.toml
COPY ./web/Cargo.toml ./web/Cargo.toml

# Copy the complete source code into the container's working directory
COPY . .

# Remove the target directory to ensure a clean build.
RUN cargo clean

# Install cross-compilation target if needed
RUN rustup target add aarch64-unknown-linux-gnu

# Build the Rust application in release mode for the ARM64 target
RUN cargo build --release --workspace --target aarch64-unknown-linux-gnu

# Stage 3: Merge the binaries
FROM debian:stable-slim AS merger

# Declare an arg for the target platform, buildx will set this value
ARG TARGETPLATFORM
ARG TARGETARCH

WORKDIR /app

# Copy binaries based on target architecture
COPY --from=builder-amd64 /usr/src/app/target/x86_64-unknown-linux-gnu/release/refactor_platform_rs /app/refactor_platform_rs
COPY --from=builder-amd64 /usr/src/app/target/x86_64-unknown-linux-gnu/release/migration /app/migration
COPY --from=builder-amd64 /usr/src/app/target/x86_64-unknown-linux-gnu/release/seed_db /app/seed_db

# Copy ARM64 binaries
COPY --from=builder-arm64 /usr/src/app/target/aarch64-unknown-linux-gnu/release/refactor_platform_rs /app/refactor_platform_rs_arm64
COPY --from=builder-arm64 /usr/src/app/target/aarch64-unknown-linux-gnu/release/migration /app/migration_arm64
COPY --from=builder-arm64 /usr/src/app/target/aarch64-unknown-linux-gnu/release/seed_db /app/seed_db_arm64

# Add entrypoint script to select the correct binary based on architecture
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
