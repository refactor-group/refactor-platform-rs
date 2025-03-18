# syntax=docker/dockerfile:1
# Specify the Dockerfile syntax version

<<<<<<< HEAD
# Stage 1: Build Stage
<<<<<<< HEAD
FROM rust:1.70-slim AS builder
=======
FROM rust:latest AS builder
=======
# Stage 1: Builder for AMD64
FROM rust:latest AS builder-amd64
>>>>>>> 88c1ea1 (refactors Dockerfile to create to builder images for compiling and adds entrypoint.sh as the entrypoint)
# AS builder names this stage for easy referencing later
>>>>>>> ca9ea8f (merges in changes from test branch.)

# Declare an arg for the target platform, buildx will set this value
ARG TARGETPLATFORM

# Declare an arg for the target architecture, buildx will set this value
ARG TARGETARCH

# Set the working directory inside the container
WORKDIR /usr/src/app
# All subsequent commands will be executed from this directory

# Install necessary packages for building Rust projects with PostgreSQL dependencies
RUN apt-get update && apt-get install -y \
    bash \
    build-essential \
    pkg-config \
    libssl-dev \
    libpq-dev  \
    --no-install-recommends \
    && rm -rf /var/lib/apt/lists/*

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
RUN if [ "$TARGETPLATFORM" = "linux/amd64" ]; then \
    rustup target add x86_64-unknown-linux-gnu; \
    fi
# Conditionally add the x86_64 target if the target platform is AMD64

# Build the Rust application in release mode for the AMD64 target
RUN if [ "$TARGETPLATFORM" = "linux/amd64" ]; then \
    cargo build --release --workspace --target x86_64-unknown-linux-gnu; \
    fi
# Conditionally build the release binary for the x86_64 target

# Stage 2: Builder for ARM64
FROM rust:latest AS builder-arm64

# Declare an arg for the target platform, buildx will set this value
ARG TARGETPLATFORM

# Declare an arg for the target architecture, buildx will set this value
ARG TARGETARCH

# Set the working directory inside the container
WORKDIR /usr/src/app
# All subsequent commands will be executed from this directory

# Install necessary packages for building Rust projects with PostgreSQL dependencies
RUN apt-get update && apt-get install -y \
    bash \
    build-essential \
    pkg-config \
    libssl-dev \
    libpq-dev  \
    --no-install-recommends \
    && rm -rf /var/lib/apt/lists/*

# Install cross-compliation target if needed
RUN if [ "$TARGETPLATFORM" = "linux/arm64" ]; then \
    rustup target add aarch64-unknown-linux-gnu; \
    fi
# Conditionally add the ARM64 target if the target platform is ARM64

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

# Build the Rust application in release mode for the ARM64 target
RUN if [ "$TARGETPLATFORM" = "linux/arm64" ]; then \
    cargo build --release --workspace --target aarch64-unknown-linux-gnu; \
    fi
# Conditionally build the release binary for the aarch64 target


# Stage 3: Merge the binaries
FROM debian:stable-slim AS merger

# Declare an arg for the target platform, buildx will set this value
ARG TARGETPLATFORM

# Declare an arg for the target architecture, buildx will set this value
ARG TARGETARCH

# Set environment variables for the build process
ENV PKG_CONFIG_ALLOW_CROSS=1
ENV OPENSSL_DIR=/usr
ENV OPENSSL_LIB_DIR=/usr/lib/aarch64-linux-gnu
ENV OPENSSL_INCLUDE_DIR=/usr/include/aarch64-linux-gnu
ENV OPENSSL_STATIC=1

# Install necessary runtime dependencies and clean up apt lists
RUN mkdir -p /usr/include/aarch64-linux-gnu && apt-get update && \
    apt-get install -y --no-install-recommends \
    libssl3 \
    libpq5 \
    libssl-dev \
    libpq-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*


# Copy the AMD64 binaries from the builder-amd64 stage
COPY --from=builder-amd64 /usr/src/app/target/x86_64-unknown-linux-gnu/release/refactor_platform_rs /usr/src/app/target/x86_64-unknown-linux-gnu/release/refactor_platform_rs
COPY --from=builder-amd64 /usr/src/app/target/x86_64-unknown-linux-gnu/release/migration /usr/src/app/target/x86_64-unknown-linux-gnu/release/migration
COPY --from=builder-amd64 /usr/src/app/target/x86_64-unknown-linux-gnu/release/seed_db /usr/src/app/target/x86_64-unknown-linux-gnu/release/seed_db

# Copy the ARM64 binaries from the builder-arm64 stage
COPY --from=builder-arm64 /usr/src/app/target/aarch64-unknown-linux-gnu/release/refactor_platform_rs /usr/src/app/target/aarch64-unknown-linux-gnu/release/refactor_platform_rs
COPY --from=builder-arm64 /usr/src/app/target/aarch64-unknown-linux-gnu/release/migration /usr/src/app/target/aarch64-unknown-linux-gnu/release/migration
COPY --from=builder-arm64 /usr/src/app/target/aarch64-unknown-linux-gnu/release/seed_db /usr/src/app/target/aarch64-unknown-linux-gnu/release/seed_db

# Set the working directory inside the container
WORKDIR /usr/src/app

# Create a non-root user for running the application
RUN useradd -m appuser && \
    chown -R appuser:appuser /usr/src/app && \
    chmod -R 755 /usr/src/app

# Switch to the non-root user
USER appuser

EXPOSE ${BACKEND_PORT}

# Create a simple script to run the correct binary based on the architecture
COPY --chmod=755 entrypoint.sh /usr/src/app/entrypoint.sh
# Copy an entrypoint script and make it executable

ENTRYPOINT ["entrypoint.sh"]
# Set the entry point to the script that selects the correct binary

