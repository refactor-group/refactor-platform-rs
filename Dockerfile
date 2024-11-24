# Stage 1: Build Stage
FROM rust:latest AS builder

# Set the working directory inside the container
WORKDIR /usr/src/app

# Install necessary packages
RUN apt-get update && apt-get install -y \
    bash \
    build-essential \
    libssl-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Install the necessary Rust target for ARM64 (Raspberry Pi 5)
RUN rustup target add aarch64-unknown-linux-gnu

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

# Build the project
RUN cargo build --release --workspace

# logs the contents of the /usr/src/app directory to the docker build log and outputs them to the console
RUN ls -la /usr/src/app/target/release/

RUN file /usr/src/app/target/release/*

# Stage 2: Runtime Stage
FROM debian:stable-slim AS runtime

# Install necessary runtime dependencies
RUN apt-get update && apt-get install -y \
    libssl3 \
    libpq5 \
    postgresql-client-15 \
    bash \
    && rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /usr/src/app

# Copy the compiled binaries from the builder stage
COPY --from=builder /usr/src/app/target/release/refactor_platform_rs /usr/local/bin/refactor_platform_rs
COPY --from=builder /usr/src/app/target/release/migration /usr/local/bin/migration
COPY --from=builder /usr/src/app/target/release/seed_db /usr/local/bin/seed_db

# Create a non-root user for running the application
RUN useradd -m appuser && \
    chown -R appuser:appuser /usr/src/app /usr/local/bin && \
    chmod +x /usr/local/bin/*

# Switch to the non-root user
USER appuser

# Expose the necessary ports
EXPOSE 4000

# Default command starts an interactive bash shell
# Set ENTRYPOINT to default to run the Rust binary with arguments
ENTRYPOINT ["/bin/bash", "-c", "/usr/local/bin/refactor_platform_rs -l DEBUG -i \"$SERVICE_INTERFACE\" -p \"$SERVICE_PORT\" -d \"$DATABASE_URL\""]

# Default CMD allows overriding with custom commands
CMD ["bash"]
