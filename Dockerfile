# syntax=docker/dockerfile:1.4

# Stage 1: Build Rust app on platform-specific image
FROM --platform=${BUILDPLATFORM} rust:bullseye AS builder

# Install required build tools
RUN apt-get update && apt-get install -y \
    build-essential bash pkg-config libssl-dev libpq-dev curl git \
    --no-install-recommends && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# Copy workspace and packages
COPY Cargo.toml Cargo.lock ./
COPY ./entity/Cargo.toml ./entity/Cargo.toml
COPY ./entity_api/Cargo.toml ./entity_api/Cargo.toml
COPY ./migration/Cargo.toml ./migration/Cargo.toml
COPY ./service/Cargo.toml ./service/Cargo.toml
COPY ./web/Cargo.toml ./web/Cargo.toml
COPY . .

RUN cargo build --release -p refactor_platform_rs -p migration

RUN echo "LIST OF CONTENTS" && ls -lahR /usr/src/app  

# Stage 2: Minimal runtime image
FROM --platform=${BUILDPLATFORM} debian:bullseye-slim

# Install Bash to support entrypoint.sh
# TODO: remove everything but bash before final commit!
RUN apt-get update && apt-get install -y bash curl wget netcat-openbsd net-tools nmap vim-tiny  && rm -rf /var/lib/apt/lists/*

# Create non-root user with 1001 UID and /bin/bash shell
RUN useradd -m -u 1001 -s /bin/bash appuser
WORKDIR /app
RUN chown appuser:appuser /app

# Copy the necessary release binaries
COPY --from=builder /usr/src/app/target/release/refactor_platform_rs .
COPY --from=builder /usr/src/app/target/release/migration ./migrationctl

# In order to run our initial migration which applies a SQL file directly, we need to
# make sure the directory exists on the container and copy the SQL file into it.
RUN mkdir -p /app/migration/src
COPY --from=builder /usr/src/app/migration/src/refactor_platform_rs.sql /app/migration/src/

# Copy entrypoint script and make it executable
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh && chown -R appuser:appuser /app /entrypoint.sh

USER appuser

EXPOSE 4000

ENTRYPOINT ["/entrypoint.sh"]
