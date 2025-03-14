# syntax=docker/dockerfile:1
# Specify the Dockerfile syntax version

# Stage 1: Build Stage
<<<<<<< HEAD
FROM rust:1.70-slim AS builder
=======
FROM rust:latest AS builder
# AS builder names this stage for easy referencing later
>>>>>>> ca9ea8f (merges in changes from test branch.)

# Set the working directory inside the container
WORKDIR /usr/src/app
# All subsequent commands will be executed from this directory

# Install necessary packages for building Rust projects with PostgreSQL dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    bash \
    build-essential \
    gcc-aarch64-linux-gnu \ 
    libssl-dev \
    pkg-config \
    libpq-dev && rm -rf /var/lib/apt/lists/*

# Add ARM64 architecture
RUN dpkg --add-architecture arm64

# Install ARM64 packages
RUN apt-get update && apt-get install -y --no-install-recommends \
    libc6-dev:arm64 \
    && rm -rf /var/lib/apt/lists/*

# Install the necessary Rust target for ARM64 (Raspberry Pi 5)
RUN rustup target add aarch64-unknown-linux-gnu

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

# Build workspace and dependencies to leverage Docker cache in release mode for ARM64
RUN cargo build --release --workspace --target aarch64-unknown-linux-gnu

# Stage 2: Runtime Stage
FROM debian:stable-slim AS runtime 

# Install necessary runtime dependencies and clean up apt lists
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 \
    libpq5 \
    && rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /usr/src/app

# Create a non-root user for running the application
RUN useradd -m appuser && \
    chown -R appuser:appuser /usr/src/app && \
    chmod -R 755 /usr/src/app

# Copy the compiled binaries from the builder stage
COPY --from=builder /usr/src/app/target/aarch64-unknown-linux-gnu/release/refactor_platform_rs /usr/local/bin/refactor_platform_rs
COPY --from=builder /usr/src/app/target/aarch64-unknown-linux-gnu/release/migration /usr/local/bin/migration
COPY --from=builder /usr/src/app/target/aarch64-unknown-linux-gnu/release/seed_db /usr/local/bin/seed_db

# Switch to the non-root user
USER appuser

# Expose the necessary ports
EXPOSE ${BACKEND_PORT}

# Set the entrypoint to run the application
ENTRYPOINT ["/bin/bash", "-c", "/usr/local/bin/refactor_platform_rs"]

# Set the default args to run when the container starts
CMD ["-l", "$BACKEND_LOG_FILTER_LEVEL", "-i", "$BACKEND_INTERFACE", "-p", "$BACKEND_PORT", "-d", "$DATABASE_URL", "--allowed-origins=$BACKEND_ALLOWED_ORIGINS"]