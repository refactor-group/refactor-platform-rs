# syntax=docker/dockerfile:1
# Specify the Dockerfile syntax version

<<<<<<< HEAD
<<<<<<< HEAD
# Stage 1: Build Stage
<<<<<<< HEAD
FROM rust:1.70-slim AS builder
=======
FROM rust:latest AS builder
=======
# Stage 1: Builder for AMD64
=======
# Stage 1: Builder Stage for AMD64
>>>>>>> 8eaac98 (update conditional compilation and renames binary locationds from architecture)
FROM rust:latest AS builder-amd64
>>>>>>> 88c1ea1 (refactors Dockerfile to create to builder images for compiling and adds entrypoint.sh as the entrypoint)
# AS builder names this stage for easy referencing later
>>>>>>> ca9ea8f (merges in changes from test branch.)

# Set the working directory inside the container
WORKDIR /usr/src/app
# All subsequent commands will be executed from this directory

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
# Copy the workspace manifest and lock file. Docker caches layers, so copying these first
# allows Docker to cache dependencies if these files don't change.

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
# AS builder names this stage for easy referencing later

# Set the working directory inside the container
WORKDIR /usr/src/app
# All subsequent commands will be executed from this directory

# Enable multiarch support
RUN dpkg --add-architecture arm64

# Update apt repositories
RUN apt-get update

# Install necessary packages for building Rust projects with PostgreSQL dependencies
RUN apt-get install -y \
    bash \
    build-essential \
    pkg-config \
    libssl-dev:arm64 \
    libpq-dev:arm64 \
    --no-install-recommends && \
    rm -rf /var/lib/apt/lists/*

# Copy the main workspace Cargo.toml and Cargo.lock to define workspace structure
COPY Cargo.toml Cargo.lock ./
# Copy the workspace manifest and lock file. Docker caches layers, so copying these first
# allows Docker to cache dependencies if these files don't change.

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

# Install cross-compliation target if needed
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

